const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const net = require('node:net');
const { spawn, execFileSync } = require('node:child_process');

const cli = path.resolve(process.env.AIO_TEST_CLI || 'target/debug/aio');
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const docker = args => execFileSync('docker', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const children = new Set(), containers = new Set();
function launch(root, extra = [], environment = {}) {
  const child = spawn(cli, ['plugin', 'dev', root, '--no-open', ...extra], { env: { ...process.env, ...environment }, stdio: ['ignore', 'pipe', 'pipe'] });
  children.add(child);
  child.text = '';
  for (const stream of [child.stdout, child.stderr]) stream.on('data', data => child.text += data);
  child.on('exit', () => children.delete(child));
  return child;
}
async function stopped(child) {
  for (let i = 0; i < 350; i++) {
    if (child.exitCode !== null || child.signalCode !== null) return;
    await delay(100);
  }
  throw new Error('沙箱没有在 35 秒内退出');
}
async function ready(root, child) {
  for (let i = 0; i < 900; i++) {
    if (child.exitCode !== null) throw new Error(child.text);
    try {
      const info = JSON.parse(await fs.readFile(path.join(root, '.aio/dev/host.json')));
      const response = await fetch(info.url + '/health', { signal: AbortSignal.timeout(2000) });
      if (i % 100 === 0 && !response.ok) console.error('宿主就绪检查', response.status());
      if (response.ok) return info;
    } catch {}
    await delay(100);
  }
  throw new Error('首次数据库和宿主没有就绪');
}
async function main() {
  const output = path.resolve('target/sandbox-acceptance');
  await fs.mkdir(output, { recursive: true });
  const directory = await fs.mkdtemp(path.join(output, 'lifecycle-'));
  const results = [];
  try {
    // 每轮创建全新的数据库卷，覆盖初始化临时 postmaster 与正式 TCP 监听的切换。
    for (let round = 0; round < 3; round++) {
      const root = path.join(directory, 'plugin-' + round);
      execFileSync(cli, ['plugin', 'init', root, '--language', 'typescript'], { stdio: 'ignore' });
      const started = Date.now();
      const child = launch(root, ['--debug']);
      const info = await ready(root, child);
      const [container] = JSON.parse(await fs.readFile(path.join(root, '.aio/dev/database.json')));
      containers.add(container);
      assert.equal(docker(['inspect', '--format', '{{.State.Running}}', container]), 'true');
      const hostMs = Date.now() - started;
      child.kill('SIGTERM');
      await stopped(child);
      assert.equal(child.exitCode, 0, child.text);
      assert.equal(docker(['inspect', '--format', '{{.State.Running}}', container]), 'false');
      await assert.rejects(fetch(info.url + '/health', { signal: AbortSignal.timeout(2000) }));
      results.push({ coldDatabase: true, hostMs, sigtermExitCode: child.exitCode, containerStopped: true, listenerClosed: true });
      if (round === 0) {
        const configuration = (await fs.readdir(path.join(root, '.run'))).find(name => name.startsWith('AIO_Frontend_'));
        assert(configuration, '没有生成浏览器调试配置');
        const file = path.join(root, '.run', configuration);
        const manual = (await fs.readFile(file, 'utf8')) + '\n<!-- developer edit -->\n';
        await fs.writeFile(file, manual);
        const warm = launch(root, ['--debug']);
        await ready(root, warm);
        await delay(300);
        warm.kill('SIGTERM');
        await stopped(warm);
        assert.equal(await fs.readFile(file, 'utf8'), manual);
        results.push({ manuallyEditedRunConfigurationPreserved: true });

        const server = net.createServer();
        await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
        try {
          const collision = launch(root, ['--port', String(server.address().port)]);
          await stopped(collision);
          assert.notEqual(collision.exitCode, 0);
          assert.equal(docker(['inspect', '--format', '{{.State.Running}}', container]), 'false');
          results.push({ occupiedPortRejected: true, failedStartupStoppedDatabase: true });
        } finally { server.close(); }

        docker(['start', container]);
        for (let i = 0; i < 60; i++) {
          try { docker(['exec', container, 'pg_isready', '-h', '127.0.0.1', '-U', 'developer']); break; }
          catch { await delay(250); }
        }
        const alternate = path.join(directory, 'another-project');
        execFileSync(cli, ['plugin', 'init', alternate, '--language', 'typescript'], { stdio: 'ignore' });
        const session = JSON.parse(await fs.readFile(path.join(root, '.aio/dev/session.json')));
        const connection = new URL(session.database_url);
        connection.host = docker(['port', container, '5432/tcp']);
        const foreign = launch(alternate, [], { AIO_DEV_DATABASE_URL: connection.href });
        await stopped(foreign);
        assert.notEqual(foreign.exitCode, 0);
        assert.match(await fs.readFile(path.join(alternate, '.aio/dev/logs/host.log'), 'utf8'), /数据库属于另一个开发沙箱/);
        assert.equal(docker(['inspect', '--format', '{{.State.Running}}', container]), 'true');
        docker(['stop', container]);
        results.push({ otherWorkspaceDatabaseRejected: true, externalDatabaseNotStopped: true });
      }
    }
    await fs.writeFile(path.join(output, 'lifecycle.json'), JSON.stringify(results, null, 2));
    console.log(JSON.stringify(results));
  } finally {
    for (const child of children) child.kill('SIGTERM');
    await Promise.all([...children].map(stopped));
    for (const project of await fs.readdir(directory)) {
      try {
        const [name] = JSON.parse(await fs.readFile(path.join(directory, project, '.aio/dev/database.json')));
        containers.add(name);
      } catch {}
    }
    for (const container of containers) {
      docker(['rm', '-f', container]);
      docker(['volume', 'rm', container]);
    }
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
