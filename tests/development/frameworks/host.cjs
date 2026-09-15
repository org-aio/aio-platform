const { createServer } = require('node:http');
const { readFile } = require('node:fs/promises');
const { resolve, extname, sep } = require('node:path');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const platform = resolve(__dirname, '../../..');
const runtime = resolve(platform, 'lib/plugin/host/src/runtime/server');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json' };

async function listen(server) {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return server.address().port;
}

exports.start = async directory => {
  const probe = createServer();
  const port = await listen(probe);
  await new Promise(resolve => probe.close(resolve));
  const child = spawn(process.execPath, ['dist/server.cjs'], {
    cwd: directory, env: { ...process.env, AIO_PLUGIN_PORT: String(port) }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const exited = once(child, 'exit');
  let log = '';
  child.stdout.on('data', data => log += data);
  child.stderr.on('data', data => log += data);
  const backend = `http://127.0.0.1:${port}`;
  let ready = false;
  for (let attempt = 0; attempt < 150; attempt++) {
    if (child.exitCode !== null) throw new Error(log);
    try { ready = (await fetch(`${backend}/health`)).ok; } catch {}
    if (ready) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (!ready) { child.kill(); throw new Error(log); }
  const bridge = await readFile(resolve(runtime, 'frontend_guest.js'));
  const lifecycle = await readFile(resolve(platform, 'sdk/web/lifecycle.js'));
  const loader = await readFile(resolve(runtime, 'vendor/es-module-shims.js'));
  const policySource = await readFile(resolve(runtime, 'frontend_document.rs'), 'utf8');
  const policy = policySource.match(/"(sandbox allow-scripts;[^"\n]+)"/)[1];
  const requests = [];
  const root = resolve(directory, 'dist/frontend');
  const server = createServer(async (request, response) => {
    try {
      const path = new URL(request.url, 'http://localhost').pathname;
      if (path === '/') {
        response.setHeader('content-type', 'text/html');
        response.end(`<!doctype html><html><body><iframe title="插件" sandbox="allow-scripts" src="/plugin/index.html" width="100%" height="700"></iframe><script>
addEventListener('message', async event => {
  const message = event.data;
  if (message.channel !== 'aio-plugin' || message.token !== 'test-token') return;
  try {
    if (message.request) {
      const request = message.request;
      const response = await fetch('/backend' + request.path, { method: request.method, body: request.body || undefined });
      event.source.postMessage({ channel: 'aio-plugin', token: message.token, id: message.id, response: { status: response.status, body: await response.text() } }, '*');
    } else if (typeof message.asset === 'string') {
      const response = await fetch('/plugin/' + message.asset);
      if (!response.ok) throw new Error('资源不存在');
      event.source.postMessage({ channel: 'aio-plugin', token: message.token, id: message.id, asset: { bytes: new Uint8Array(await response.arrayBuffer()), type: response.headers.get('content-type') } }, '*');
    }
  } catch(error) { event.source.postMessage({ channel:'aio-plugin', token:message.token, id:message.id, error:String(error) }, '*'); }
});</script></body></html>`);
        return;
      }
      if (path.startsWith('/backend/')) {
        const chunks = [];
        for await (const data of request) chunks.push(data);
        const target = await fetch(backend + path.slice('/backend'.length), {
          method: request.method, headers: { 'content-type': 'application/json', 'x-aio-tenant-id': 'browser-tenant' },
          body: request.method === 'GET' ? undefined : Buffer.concat(chunks),
        });
        requests.push({ path, status: target.status });
        response.writeHead(target.status, { 'content-type': 'application/json' });
        response.end(await target.text());
        return;
      }
      response.setHeader('content-type', types[extname(path)] || 'application/octet-stream');
      if (path === '/plugin/__aio_bridge.js') { response.end(Buffer.concat([lifecycle, bridge])); return; }
      if (path === '/plugin/__aio_modules.js') { response.end(loader); return; }
      const asset = resolve(root, decodeURIComponent(path.slice('/plugin/'.length)));
      if (!path.startsWith('/plugin/') || !asset.startsWith(root + sep)) { response.writeHead(404).end(); return; }
      let content = await readFile(asset);
      if (path === '/plugin/index.html') {
        const prefix = `http://127.0.0.1:${server.address().port}/plugin/`;
        response.setHeader('content-security-policy', policy.replaceAll('{prefix}', prefix));
        content = content.toString().replace('<head>', `<head><base href="${prefix}"><script src="${prefix}__aio_bridge.js" data-token="test-token"></script><script src="${prefix}__aio_modules.js"></script>`)
          .replaceAll('type="module"', 'type="module-shim"').replaceAll('type="importmap"', 'type="importmap-shim"');
      }
      response.end(content);
    } catch (error) { response.writeHead(404).end(String(error)); }
  });
  const address = `http://127.0.0.1:${await listen(server)}`;
  return {
    address, requests,
    async close() {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
      child.kill('SIGTERM');
      const timeout = setTimeout(() => child.kill('SIGKILL'), 5000);
      await exited;
      clearTimeout(timeout);
    },
  };
};
