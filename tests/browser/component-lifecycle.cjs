const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { once } = require('node:events');
const { readFile } = require('node:fs/promises');
const { chromium } = require('playwright');

const lifecycle = readFile('sdk/web/lifecycle.js', 'utf8');
const guest = readFile('sdk/web/guest.js', 'utf8');
const bridge = readFile('sdk/web/host.mjs', 'utf8');
const catalogWatch = readFile('lib/plugin/host/src/runtime/catalog_watch.js', 'utf8');
const component = Promise.all([
  readFile('lib/plugin/host/src/runtime/frontend_lifecycle.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_cache.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_assets.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_navigation.js', 'utf8'),
  readFile('lib/plugin/host/src/runtime/frontend_component.js', 'utf8'),
]).then(parts => parts.join('\n'));

let origin;
let sequence = 0;
const grants = new Map();
const counts = { mount: 0, delete: 0, request: 0, manage: 0, action: 0, image: 0, renew: 0 };
const requests = [];
const management = [];
const imageTickets = [];
const streamTickets = [];
let deleteDelay = 0;
let actionDelay = 0;
let rejectedRenewal;

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
      if (grants.size >= 1) return send(response, 400, { error: '当前用户的前端挂载数量超过配额' });
      const token = `ticket-${++sequence}`;
      const mount = { abi: 2, token, revision: 'revision-a', generation: 'generation-a', session_context: 'session-a', context: 'tenant-a', page_id: payload.page_id, assets: {}, src: `/api/runtime/components/assets/${token}/index.html` };
      grants.set(token, mount);
      counts.mount++;
      return send(response, 200, { data: mount });
    }
    const revoke = path.match(/^\/api\/runtime\/(?:frontend|components)\/([^/]+)$/);
    if (revoke && request.method === 'DELETE') {
      counts.delete++;
      if (deleteDelay) await new Promise(resolve => setTimeout(resolve, deleteDelay));
      grants.delete(revoke[1]);
      response.writeHead(204);
      return response.end();
    }
    const requestRoute = path.match(/^\/api\/runtime\/components\/([^/]+)\/(request|manage|action|image|renew)$/);
    if (requestRoute) {
      const [, token, action] = requestRoute;
      if (action === 'renew') counts.renew++;
      if (token === rejectedRenewal) return send(response, 403, { error: 'Mount expired' });
      if (!grants.has(token)) return send(response, 403, { error: 'Mount revoked' });
      if (action === 'renew') {
        response.writeHead(204);
        return response.end();
      }
      counts[action]++;
      if (action === 'image') {
        imageTickets.push(token);
        response.writeHead(200, { 'content-type': 'image/png' });
        return response.end(Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aV8kAAAAASUVORK5CYII=', 'base64'));
      }
      if (action === 'action') {
        const approved = JSON.parse((await body(request)).toString());
        if (actionDelay) await new Promise(resolve => setTimeout(resolve, actionDelay));
        return send(response, 200, { data: approved });
      }
      if (action === 'manage') management.push({ token, request: JSON.parse((await body(request)).toString()) });
      else requests.push(token);
      return send(response, 200, { data: { status: 200, headers: [], body: Array.from(Buffer.from('{}')) } });
    }
    const stream = path.match(/^\/api\/runtime\/components\/([^/]+)\/download$/);
    if (stream) {
      if (!grants.has(stream[1])) return send(response, 403, {error:'Mount revoked'});
      const parameters = new URL(request.url, origin).searchParams;
      assert.equal(parameters.get('path'), '/files/large');
      assert.equal(parameters.get('name'), 'large.bin');
      assert.equal(parameters.get('query'), 'version=2');
      streamTickets.push(stream[1]);
      response.writeHead(200, {'content-type':'application/octet-stream', 'content-disposition':'attachment; filename="large.bin"', 'content-length':100 * 1024 * 1024});
      for (let index = 0; index < 100; index++) {
        if (!response.write(Buffer.alloc(1024 * 1024, 0x5a))) await once(response, 'drain');
      }
      return response.end();
    }
    const download = path.match(/^\/api\/runtime\/components\/([^/]+)\/packages\/([a-f0-9]{64})$/);
    if (download) {
      if (!grants.has(download[1])) return send(response, 403, { error: 'Mount revoked' });
      response.writeHead(200, { 'content-type': 'application/vnd.aio.component+gzip', 'content-disposition': `attachment; filename="${download[2]}.aio-plugin"` });
      return response.end(Buffer.alloc(34 * 1024 * 1024, 0xa5));
    }
    const asset = path.match(/^\/api\/runtime\/components\/assets\/([^/]+)\/index\.html$/);
    if (asset) {
      if (!grants.has(asset[1])) return send(response, 401, { error: 'Mount missing' });
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      return response.end(`<!doctype html><html><body><script data-token="${asset[1]}">${await lifecycle}</script><script>${await guest}</script><button id="download">Download</button><button id="stream">Stream</button><button id="settings">Settings</button><button id="image">Image</button><img id="document-image"><script>
      window.marker = 'initial';
      document.querySelector('#image').onclick = async () => {
        try {
          const response = await window.aioPlugin.managementImage('/api/runtime/marketplace/' + 'a'.repeat(64) + '/images/image.png');
          if (response.status !== 200) throw new Error('Image unavailable');
          const image = document.querySelector('#document-image');
          if (image.src) URL.revokeObjectURL(image.src);
          image.src = URL.createObjectURL(new Blob([response.body], { type: response.headers[0].value }));
        } catch (error) { window.imageError = error.message; }
      };
      document.querySelector('#stream').onclick = () => window.aioPlugin.downloadFrom('/files/large', 'large.bin', 'version=2').catch(error => window.streamError = error.message);
      document.querySelector('#download').onclick = () => window.aioPlugin.downloadPackage('a'.repeat(64)).catch(error => window.actionError = error.message);
      document.querySelector('#settings').onclick = () => window.aioPlugin.openSettings('component:other:settings').catch(error => window.actionError = error.message);
      </script></body></html>`);
    }
    send(response, 404, { error: 'Not found' });
  } catch (error) {
    send(response, 500, { error: error.message });
  }
});

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ executablePath: process.env.AIO_BROWSER_EXECUTABLE, headless: true });
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.goto(origin);
    const mount = await page.evaluate(async () => (await (await fetch('/api/runtime/frontend/mount', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ page_id: 'component-page' }) })).json()).data);
    await page.evaluate(config => {
      document.body.innerHTML = `<div id="page" data-aio-page-active="true" data-aio-workspace-active="true" data-aio-workspace-context="${config.context}"><div class="application-frontend-shell"><div id="frontend-loading" class="application-frontend-loading"></div><iframe id="frame" sandbox="allow-scripts allow-forms"></iframe></div></div>`;
      window.__hostMessages = [];
      window.__catalogChanges = 0;
      window.__catalogRequests = [];
      window.__settings = [];
      window.addEventListener('aio:plugin-settings', event => window.__settings.push(event.detail.pageId));
      let receive = 0;
      window.dioxus = {
        recv: () => receive++ === 0 ? Promise.resolve({ ...config, id: 'frame', development: false }) : new Promise(resolve => { window.__finish = resolve; }),
        send: message => {
          window.__hostMessages.push(message);
          if (message?.ready === true) document.querySelector('#frontend-loading')?.remove();
        },
      };
    }, mount);
    await page.evaluate(source => {
      const watcher = {
        send: requests => {
          if (requests === null) return;
          window.__catalogChanges++;
          window.__catalogRequests = requests;
          if (!window.__holdCatalog) requestAnimationFrame(() => window.dispatchEvent(new CustomEvent('aio:catalog-refreshed', { detail: { requests, success: true } })));
        },
        recv: () => new Promise(resolve => { window.__stopCatalog = resolve; }),
      };
      void new Function('dioxus', `return (async () => { ${source} })();`)(watcher);
    }, await catalogWatch);
    await page.addScriptTag({ content: `(async () => {\n${await component}\n})().catch(error => { window.__hostMessages.push({ error: error.message }); });` });

    const frame = page.frameLocator('#frame');
    await page.waitForFunction(() => document.querySelector('#frontend-loading') === null);
    assert.equal(grants.size, 1);
    const initialSource = await page.locator('#frame').getAttribute('src');
    const manage = method => frame.locator('body').evaluate(async (_, method) => {
      const path = method === 'GET' ? '/api/runtime/catalog' : '/api/runtime/plugins/install';
      const response = await window.aioPlugin.manage({ method, path, body: new Uint8Array(), headers: [] });
      return { status: response.status, typedBody: response.body instanceof Uint8Array };
    }, method);
    assert.deepEqual(await manage('GET'), { status: 200, typedBody: true });
    assert.equal(management[0].token, mount.token);
    assert.equal(counts.request, 0);
    assert.equal(await page.evaluate(() => window.__catalogChanges), 0);
    await page.evaluate(() => { window.__holdCatalog = true; });
    let managed = false;
    const writing = manage('POST').then(value => { managed = true; return value; });
    await page.waitForFunction(() => window.__catalogChanges === 1);
    assert.equal(managed, false, '管理调用必须等待宿主完成目录更新');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('aio:catalog-refreshed', { detail: { requests: ['older-request'], success: true } })));
    await page.waitForTimeout(50);
    assert.equal(managed, false, '更早的目录刷新不能确认本次写入');
    let nextManaged = false;
    const nextWriting = manage('POST').then(() => { nextManaged = true; return null; }, error => error.message);
    await page.waitForFunction(() => window.__catalogChanges === 2);
    assert.equal(await page.evaluate(() => window.__catalogRequests.length), 2, '刷新期间的并发请求不能丢失');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('aio:catalog-refreshed', { detail: { requests: [window.__catalogRequests[0]], success: true } })));
    await writing;
    assert.equal(nextManaged, false, '前一次完成不能放行后一次写入');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('aio:catalog-refreshed', { detail: { requests: window.__catalogRequests, success: false } })));
    assert.match(await nextWriting, /操作已提交.*更新插件目录失败/);
    await page.evaluate(() => { window.__holdCatalog = false; window.dispatchEvent(new CustomEvent('aio:catalog-refreshed', { detail: { requests: window.__catalogRequests, success: true } })); });

    await frame.locator('#image').click();
    await frame.locator('#document-image[src^="blob:"]').waitFor({ state: 'attached' });
    await frame.locator('#document-image').evaluate(image => image.decode());
    assert.equal(await frame.locator('#document-image').evaluate(image => image.naturalWidth), 1);
    assert.equal(imageTickets.at(-1), mount.token);
    const downloading = page.waitForEvent('download');
    await frame.locator('#download').click();
    const downloaded = await downloading;
    assert.equal(downloaded.suggestedFilename(), `${'a'.repeat(64)}.aio-plugin`);
    assert.equal((await readFile(await downloaded.path())).length, 34 * 1024 * 1024);
    const streaming = page.waitForEvent('download');
    await frame.locator('#stream').click();
    const streamed = await streaming;
    assert.equal(streamed.suggestedFilename(), 'large.bin');
    const streamedBytes = await readFile(await streamed.path());
    assert.equal(streamedBytes.length, 100 * 1024 * 1024);
    assert(streamedBytes.every(byte => byte === 0x5a));
    assert.equal(streamTickets.at(-1), mount.token);
    await frame.locator('#settings').click();
    await page.waitForFunction(() => window.__settings.length === 1);
    assert.equal(await page.evaluate(() => window.__settings[0]), 'component:other:settings');

    await page.evaluate(() => { window.__holdCatalog = true; });
    const interruptedWrite = manage('POST').then(() => null, error => error.message);
    await page.waitForFunction(() => window.__catalogChanges === 3);
    assert.equal(await page.evaluate(() => window.__catalogRequests.length), 1, '已确认请求必须从待更新集合移除');
    deleteDelay = 250;
    await page.evaluate(() => document.querySelector('#page').dataset.aioWorkspaceActive = 'false');
    await waitFor(() => counts.delete === 1);
    assert.match(await interruptedWrite, /暂停|替换/);
    await page.evaluate(() => { window.__holdCatalog = false; });
    assert.equal(await page.locator('#frame').getAttribute('src'), initialSource);
    assert.equal(await frame.locator('body').evaluate(() => window.marker), 'initial');
    const beforeStreams = streamTickets.length;
    await frame.locator('#stream').click();
    await frame.locator('body').evaluate(() => window.aioPlugin.downloadFrom('/files/large', 'large.bin').then(() => { throw new Error('暂停下载未拒绝'); }, error => { if (!/就绪/.test(error.message)) throw error; }));
    assert.equal(streamTickets.length, beforeStreams);
    const beforeSuspended = counts.manage;
    await assert.rejects(manage('GET'), /暂停/);
    assert.equal(counts.manage, beforeSuspended);

    await page.evaluate(() => document.querySelector('#page').dataset.aioWorkspaceActive = 'true');
    await page.waitForTimeout(100);
    assert.equal(counts.mount, 1);
    await waitFor(() => counts.mount === 2 && grants.size === 1);
    deleteDelay = 0;
    await waitFor(() => counts.mount === 2 && grants.size === 1);
    const restoredTicket = [...grants.keys()][0];
    assert.notEqual(restoredTicket, mount.token);
    await frame.locator('body').evaluate(() => window.aioPlugin.request({ method: 'GET', path: '/tasks' }));
    await waitFor(() => requests.includes(restoredTicket));
    await manage('GET');
    assert.equal(management.at(-1).token, restoredTicket);

    rejectedRenewal = restoredTicket;
    counts.renew = 0;
    await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
    await waitFor(() => counts.renew === 1);
    await waitFor(() => grants.size === 1 && !grants.has(restoredTicket));
    const renewedTicket = [...grants.keys()][0];
    assert.notEqual(renewedTicket, restoredTicket);
    assert.equal(await page.locator('#frame').getAttribute('src'), initialSource);
    assert.equal(await frame.locator('body').evaluate(() => window.marker), 'initial');
    await frame.locator('body').evaluate(() => window.aioPlugin.request({ method: 'GET', path: '/tasks' }));
    await waitFor(() => requests.includes(renewedTicket));
    await manage('GET');
    assert.equal(management.at(-1).token, renewedTicket);
    rejectedRenewal = undefined;
    const restoredDownload = page.waitForEvent('download');
    await frame.locator('#stream').click();
    assert.equal((await readFile(await (await restoredDownload).path())).length, 100 * 1024 * 1024);
    assert.equal(streamTickets.at(-1), renewedTicket);
    const previousImage = await frame.locator('#document-image').getAttribute('src');
    await frame.locator('#image').click();
    await waitFor(() => imageTickets.at(-1) === renewedTicket);
    await frame.locator(`#document-image[src^="blob:"]:not([src="${previousImage}"])`).waitFor({ state: 'attached' });
    await frame.locator('#document-image').evaluate(image => image.decode());

    actionDelay = 500;
    const actionsBefore = counts.action;
    await frame.locator('#settings').click();
    await waitFor(() => counts.action === actionsBefore + 1);
    await page.evaluate(() => document.querySelector('#page').dataset.aioWorkspaceActive = 'false');
    await waitFor(() => grants.size === 0);
    await page.waitForTimeout(600);
    assert.equal(await page.evaluate(() => window.__settings.length), 1);
    assert(await frame.locator('body').evaluate(() => Boolean(window.actionError)));
    actionDelay = 0;
    await page.evaluate(() => document.querySelector('#page').dataset.aioWorkspaceActive = 'true');
    await waitFor(() => grants.size === 1);

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
    await page.evaluate(() => window.__stopCatalog());
    console.log(JSON.stringify({ streamingDownload100MiB: true, streamingDownloadRevoked: true, renewedDownload: true, retainedFrame: true, releasedOnSuspend: true, restoredWithNewGrant: true, expiredLeaseRecoveredInPlace: true, repeatedCycles: 20, bfcache: true, maxGrants: 1, management: { isolated: true, suspended: true, renewed: true, catalogInvalidated: true, largeDownload: true, settings: true, suspendedActionCancelled: true, authorizedImage: true, renewedImage: true } }));
  } finally {
    await context.close();
    await browser.close();
  }
})().then(() => server.close()).catch(error => {
  console.error(error);
  server.close();
  process.exitCode = 1;
});
