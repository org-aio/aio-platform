const fs = require('node:fs/promises');
const net = require('node:net');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, spawnSync } = require('node:child_process');
const { transport } = require('./transport.cjs');

async function main() {
  const [image, artifact, bridgeImage] = process.argv.slice(2);
  if (!image || !artifact || !bridgeImage) throw new Error('开发容器启动参数不完整');
  const config = JSON.parse(await fs.readFile(process.env.AIO_PLUGIN_CONFIG));
  const socketPath = process.env.AIO_PLUGIN_SOCKET;
  const id = 'aio-dev-' + crypto.randomUUID();
  const serviceName = id + '-service', bridgeName = id + '-bridge';
  let finished = false, service, bridge, server, channel;
  function cleanup(code = 0) {
    if (finished) return;
    finished = true;
    server?.close(); channel?.close();
    spawnSync('docker', ['rm', '-f', serviceName, bridgeName], { stdio: 'ignore', timeout: 10000 });
    spawnSync('docker', ['volume', 'rm', id], { stdio: 'ignore', timeout: 10000 });
    process.exit(code);
  }
  process.once('exit', () => {
    if (!finished) {
      spawnSync('docker', ['rm', '-f', serviceName, bridgeName], { stdio: 'ignore', timeout: 10000 });
      spawnSync('docker', ['volume', 'rm', id], { stdio: 'ignore', timeout: 10000 });
    }
  });
  process.on('SIGINT', () => cleanup()); process.on('SIGTERM', () => cleanup());
  process.on('uncaughtException', error => { console.error(error.message); cleanup(1); });
  process.on('unhandledRejection', error => { console.error(error.message); cleanup(1); });
  const volume = spawnSync('docker', ['volume', 'create', id], { encoding: 'utf8' });
  if (volume.status !== 0) throw new Error('Docker 尚未就绪，请启动本地 Docker 引擎');
  bridge = spawn('docker', ['run', '--rm', '-i', '--name', bridgeName, '--platform=linux/amd64', '--network=none', '--read-only', '--cpus=1', '--memory=256m',
    '-v', `${id}:/sandbox`, '-v', `${__dirname}:/bridge:ro`, '--entrypoint=node', bridgeImage, '/bridge/worker.cjs'], { stdio: ['pipe', 'pipe', 'inherit'] });
  bridge.on('error', error => { console.error(error.message); cleanup(1); });
  bridge.on('exit', code => cleanup(code || 1));
  const database = config.database_url ? new URL(config.database_url) : null;
  channel = transport(bridge.stdout, bridge.stdin, destination => {
    if (destination === 'broker') return net.connect(config.broker_socket);
    if (destination === 'database' && database) {
      const directory = database.searchParams.get('host');
      return directory?.startsWith('/') ? net.connect(path.join(directory, `.s.PGSQL.${database.port || 5432}`))
        : net.connect({ host: database.hostname, port: Number(database.port || 5432) });
    }
    throw new Error('未声明的宿主目标');
  }, 1);
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('开发容器传输启动超时')), 60000);
    channel.onControl(value => { if (value.ready) { clearTimeout(timer); resolve(); } });
    void channel.control(config);
  });
  server = net.createServer(socket => channel.open(socket, 'service'));
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(socketPath, resolve); });
  service = spawn('docker', ['run', '--rm', '--init', '--name', serviceName, '--platform=linux/amd64', '--network=none', '--read-only', '--user=65532:65532', '--cpus=2', '--memory=2g',
    '-v', `${id}:/sandbox`, '-v', `${artifact}:/aio/server:ro`, '-e', 'AIO_PLUGIN_CONFIG=/sandbox/config.json', '-e', 'AIO_PLUGIN_SOCKET=/sandbox/service.sock', '--entrypoint=/aio/server', image], { stdio: 'inherit' });
  service.on('error', error => { console.error(error.message); cleanup(1); });
  service.on('exit', code => cleanup(code || 0));
}
main().catch(error => { console.error(error.message); process.exit(1); });
