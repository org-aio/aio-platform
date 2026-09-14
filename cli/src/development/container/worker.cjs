const fs = require('node:fs/promises');
const net = require('node:net');
const { transport } = require('./transport.cjs');
const channel = transport(process.stdin, process.stdout, destination => {
  if (destination !== 'service') throw new Error('未声明的容器目标');
  return net.connect('/sandbox/service.sock');
}, 2);
let initialized = false;
channel.onControl(async config => {
  try {
    if (initialized) throw new Error('重复初始化容器');
    initialized = true;
    await fs.chmod('/sandbox', 0o777);
    config.broker_socket = '/sandbox/broker.sock';
    const listen = async (path, destination) => {
      const server = net.createServer(socket => channel.open(socket, destination));
      await new Promise((resolve, reject) => { server.once('error', reject); server.listen(path, resolve); });
      await fs.chmod(path, 0o666);
    };
    await listen(config.broker_socket, 'broker');
    if (config.database_url) {
      const url = new URL(config.database_url);
      url.hostname = 'localhost'; url.port = '5432';
      url.searchParams.set('host', '/sandbox');
      config.database_url = url.toString();
      await listen('/sandbox/.s.PGSQL.5432', 'database');
    }
    await fs.writeFile('/sandbox/config.json', JSON.stringify(config), { mode: 0o644 });
    await channel.control({ ready: true });
  } catch (error) { console.error(error.message); process.exit(1); }
});
process.stdin.on('end', () => process.exit(0));
