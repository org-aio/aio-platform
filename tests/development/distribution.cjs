const fs = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require('@playwright/test');

const cli = process.env.AIO_TEST_CLI;
if (!cli) throw new Error('AIO_TEST_CLI 必须指向已安装的 CLI 分发入口');
const output = path.resolve('target/distribution-acceptance');
const environment = { ...process.env };
for (const key of ['AIO_DEV_HOST', 'AIO_DEV_WEB_DIST', 'AIO_DEV_DATABASE_URL', 'AIO_DEV_CATALOG_SESSION']) delete environment[key];
const processes = new Set();
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function stop(child) {
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
  for (let i = 0; i < 350; i++) {
    if (child.exitCode !== null || child.signalCode !== null) return;
    await delay(100);
  }
  throw new Error('已安装的 npm 入口没有完成退出清理');
}
async function start(project, label) {
  const started = Date.now();
  const log = await fs.open(path.join(output, label + '.log'), 'w');
  const child = spawn(cli, ['plugin', 'dev', project, '--no-open'], { env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
  processes.add(child);
  let text = '', finished = false;
  for (const stream of [child.stdout, child.stderr]) stream.on('data', data => { text += data; void log.write(data); });
  child.once('exit', () => { processes.delete(child); finished = true; void log.close(); });
  for (let i = 0; i < 18000; i++) {
    if (finished) throw new Error(label + ' 提前退出，查看运行日志');
    if (text.includes('已加载 ')) {
      const { url } = JSON.parse(await fs.readFile(path.join(project, '.aio/dev/host.json')));
      return { child, url, readyMs: Date.now() - started };
    }
    await delay(100);
  }
  throw new Error(label + ' 30 分钟内未完成首次构建');
}
async function verify(browser, language, run, label) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.goto(run.url);
    const frame = page.frameLocator('iframe');
    if (language === 'kotlin') {
      const request = page.waitForResponse(response => response.url().endsWith('/request') && response.status() === 200, { timeout: 90000 });
      await frame.getByRole('button', { name: 'Counter', exact: true }).click({ force: true, timeout: 90000 });
      await request;
      await frame.getByRole('button', { name: '+1', exact: true }).click({ force: true });
      await frame.getByText('1', { exact: true }).waitFor();
    } else {
      await frame.getByRole('button', { name: '请求后端 +1', exact: true }).click({ timeout: 90000 });
      await frame.getByText(language === 'rust' ? '服务端结果：1' : '1', { exact: true }).first().waitFor({ timeout: 60000 });
    }
    assert.equal(await page.locator('iframe').count(), 1);
    assert.deepEqual(errors, []);
    await page.screenshot({ path: path.join(output, label + '.png') });
  } finally { await page.goto('about:blank'); await page.close(); }
}
async function main() {
  await fs.mkdir(output, { recursive: true });
  const projects = await fs.mkdtemp(path.join(output, 'projects-'));
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const results = [];
  try {
    for (const language of ['typescript', 'kotlin', 'rust']) {
      const project = path.join(projects, language);
      execFileSync(cli, ['plugin', 'init', project, '--language', language, '--name', 'distribution-' + language], { env: environment, stdio: 'ignore' });
      const cold = await start(project, language + '-cold');
      await verify(browser, language, cold, language + '-cold');
      await stop(cold.child);
      const warm = await start(project, language + '-warm');
      await verify(browser, language, warm, language + '-warm');
      await stop(warm.child);
      const lock = JSON.parse(await fs.readFile(path.join(project, 'aio-dev.lock')));
      assert.equal(lock.plugins.length, 1);
      assert.equal(lock.plugins[0].source_sha, null);
      results.push({ language, hostVersion: lock.host_version, project, freshProjectMs: cold.readyMs, cachedRestartMs: warm.readyMs, noRemote: true, bundledHost: true, frontendBackend: true });
      await fs.writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2));
      console.log(JSON.stringify(results.at(-1)));
    }
  } finally {
    await browser.close();
    await Promise.all([...processes].map(stop));
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
