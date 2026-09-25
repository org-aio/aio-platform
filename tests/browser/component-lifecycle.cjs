const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { readFile } = require('node:fs/promises');
const { randomUUID } = require('node:crypto');
const { chromium } = require('playwright');

const lifecycle = readFile('sdk/web/lifecycle.js', 'utf8');
const bridge = readFile('sdk/web/host.mjs', 'utf8');
const component = Promise.all([
  readFile('lib/plugin/host/src/runtime/frontend_lifecycle.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_cache.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_assets.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_component.js', 'utf8'),
]).then(parts => parts.join('\n'));

let origin;
let sequence = 0;
const grants = new Map();
const counts = { mount: 0, delete: 0, request: 0, renew: 0 };
const requests = [];

function send(response, status, data) {
  response.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' });
  response.end(JSON.stringify(data));
}

async function body(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks);
}

async function waitFor(predicate) {
  for (let attempt = 0; attempt < 200; attempt++) {
    if (predicate()) return;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.fail('等待组件票据生命周期状态超时');
}

const server = createServer(async (request, response) => {
  try {
    const path = decodeURIComponent(new URL(request.url, origin).pathname);
    if (path === '/') {
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      return response.end('<!doctype html><html><body></body></html>');
    }
    if (path === '/api/runtime/components/bridge.js') {
      response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8' });
      return response.end(await bridge);
    }
    if (path === '/api/runtime/frontend/mount' && request.method === 'POST') {
      const payload = JSON.parse((await body(request)).toString() || '{}');
      const token = `ticket-${++sequence}`;
      const mount = { abi: 2, token, revision: 'revision-a', generation: 'generation-a', session_context: 'session-a', context: 'tenant-a', page_id: payload.page_id, assets: {}, src: `/api/runtime/components/assets/${token}/index.html` };
      grants.set(token, mount);
      counts.mount++;
      return send(response, 200, { data: mount });
    }
    const revoke = path.match(/^\/api\/runtime\/(?:frontend|components)\/([^/]+)$/);
    if (revoke && request.method === 'DELETE') {
      counts.delete++;
      grants.delete(revoke[1]);
      response.writeHead(204);
      return response.end();
    }
    const requestRoute = path.match(/^\/api\/runtime\/components\/([^/]+)\/(request|renew)$/);
    if (requestRoute) {
      const [, token, action] = requestRoute;
      if (!grants.has(token)) return send(response, 403, { error: 'Mount revoked' });
      if (action === 'renew') {
        counts.renew++;
        response.writeHead(204);
        return response.end();
      }
      counts.request++;
      requests.push(token);
      return send(response, 200, { data: { status: 200, headers: [], body: Array.from(Buffer.from('{}')) } });
    }
    const asset = path.match(/^\/api\/runtime\/components\/assets\/([^/]+)\/index\.html$/);
    if (asset) {
      if (!grants.has(asset[1])) return send(response, 401, { error: 'Mount missing' });
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      return response.end(`<!doctype html><html><body><script data-token="${asset[1]}">${await lifecycle}</script><script>window.marker = 'initial'; addEventListener('message', event => { if (event.source !== parent || event.data?.channel !== 'test-request') return; parent.postMessage({ protocol: 'aio:plugin@2', kind: 'request', id: event.data.id, request: { method: 'GET', path: '/tasks', query: null, body: new Uint8Array() } }, '*'); });</script></body></html>`);
    }
    send(response, 404, { error: 'Not found' });
  } catch (error) {
    send(response, 500, { error: error.message });
  }
});

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.goto(origin);
    const mount = await page.evaluate(async () => (await (await fetch('/api/runtime/frontend/mount', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ page_id: 'component-page' }) })).json()).data);
    await page.evaluate(config => {
      document.body.innerHTML = `<div id="page" data-aio-page-active="true" data-aio-workspace-active="true" data-aio-workspace-context="${config.context}"><iframe id="frame" sandbox="allow-scripts allow-forms"></iframe></div>`;
      window.__hostMessages = [];
      let receive = 0;
      window.dioxus = {
        recv: () => receive++ === 0 ? Promise.resolve({ ...config, id: 'frame', development: false }) : new Promise(resolve => { window.__finish = resolve; }),
        send: message => window.__hostMessages.push(message),
      };
    }, mount);
    await page.addScriptTag({ content: `(async () => {\n${await component}\n})().catch(error => { window.__hostMessages.push({ error: error.message }); });` });

    const frame = page.frameLocator('#frame');
    await frame.locator('body').waitFor({ state: 'attached' });
    await page.waitForFunction(() => document.querySelector('#frame')?.dataset.aioPrepared === undefined || true);
    assert.equal(grants.size, 1);
    const initialSource = await page.locator('#frame').getAttribute('src');

    await page.evaluate(() => document.querySelector('#page').dataset.aioWorkspaceActive = 'false');
    await waitFor(() => counts.delete === 1 && grants.size === 0);
    assert.equal(await page.locator('#frame').getAttribute('src'), initialSource);
    assert.equal(await frame.locator('body').evaluate(() => window.marker), 'initial');

    await page.evaluate(() => document.querySelector('#page').dataset.aioWorkspaceActive = 'true');
    await waitFor(() => counts.mount === 2 && grants.size === 1);
    const restoredTicket = [...grants.keys()][0];
    assert.notEqual(restoredTicket, mount.token);
    await page.evaluate(() => document.querySelector('#frame').contentWindow.postMessage({ channel: 'test-request', id: 'request-after-restore' }, '*'));
    await waitFor(() => requests.includes(restoredTicket));

    for (let index = 0; index < 20; index++) {
      await page.evaluate(active => document.querySelector('#page').dataset.aioWorkspaceActive = String(active), false);
      await waitFor(() => grants.size === 0);
      assert(grants.size <= 1);
      await page.evaluate(active => document.querySelector('#page').dataset.aioWorkspaceActive = String(active), true);
      await waitFor(() => grants.size === 1);
      assert(grants.size <= 1);
    }
    assert.equal(await page.locator('#frame').getAttribute('src'), initialSource);
    assert.equal(await frame.locator('body').evaluate(() => window.marker), 'initial');

    await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true })));
    await waitFor(() => grants.size === 0);
    await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true })));
    await waitFor(() => grants.size === 1);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ retainedFrame: true, releasedOnSuspend: true, restoredWithNewGrant: true, repeatedCycles: 20, bfcache: true, maxGrants: 1 }));
  } finally {
    await context.close();
    await browser.close();
  }
})().then(() => server.close()).catch(error => {
  console.error(error);
  server.close();
  process.exitCode = 1;
});
