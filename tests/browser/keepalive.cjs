const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { readFile, realpath, mkdir, writeFile, readdir } = require('node:fs/promises');
const { resolve, extname, sep } = require('node:path');
const { randomUUID, createHash } = require('node:crypto');
const { chromium } = require('playwright');
const { PNG } = require('pngjs');
const { parse, serialize } = require('parse5');

const output = resolve('target/keepalive-test');
const shell = resolve(process.env.AIO_TEST_SHELL || 'target/dx/aio-host/release/web/public');
const frontend = resolve(process.env.AIO_TEST_KMP_FRONTEND || '../aio-plugin-kmp-example/dist/frontend');
const guest = Promise.all([readFile('sdk/web/lifecycle.js', 'utf8'), readFile('lib/plugin/host/src/runtime/server/frontend_guest.js', 'utf8')]).then(parts => parts.join('\n'));
const componentGuest = Promise.all([...['lifecycle.js', 'wasm.js', 'guest.js'].map(name => readFile(`sdk/web/${name}`, 'utf8')), readFile('lib/plugin/host/src/runtime/server/components/frontend_assets.js', 'utf8')]).then(parts => parts.join('\n'));
const modules = readFile('lib/plugin/host/src/runtime/server/vendor/es-module-shims.js');
const assets = (async () => {
  const entries = await readdir(frontend, { recursive: true, withFileTypes: true });
  return Object.fromEntries(await Promise.all(entries.filter(entry => entry.isFile()).map(async entry => {
    const path = resolve(entry.parentPath, entry.name);
    return [path.slice(frontend.length + 1), createHash('sha256').update(await readFile(path)).digest('hex')];
  })));
})();
let fixture;
let abi = 1;
let origin;
const grants = new Map();
const counts = { mount: 0, delete: 0, asset: 0, renew: 0, business: 0 };
const screen = (id, body = { kind: 'text', title: id, content: id }) => ({
  id, label: id, scene: { id: 'community', label: '社区插件' }, menu_path: [],
  required_permission: null, body,
});
function reset() {
  grants.clear();
  Object.keys(counts).forEach(key => counts[key] = 0);
  fixture = {
    session: { user_id: 'tester', account: 'tester', display_name: 'Tester', tenant_id: 'test', tenant_label: 'Test', permissions: [] },
    catalog: {
      session_context: 'login-a', context: 'session-a', tenant: { id: 'test', label: 'Test' },
      user: { label: 'Tester', handle: '@tester', initials: 'T' }, plugins: [],
      pages: [{ ...screen('测试工作区'), scene: { id: 'workspace', label: '工作区' } }, screen('Compose 保活', { kind: 'frontend', entry: 'index.html' }), ...Array.from({ length: 7 }, (_, i) => screen(`页面 ${i}`)), screen('独立账户页')],
      page_versions: { 'Compose 保活': 'version-a' },
      account_items: [{ id: 'test-account', label: '独立账户页', page_id: '独立账户页', required_permission: null }],
    },
  };
}
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.otf': 'font/otf', '.ttf': 'font/ttf', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
async function file(root, path) {
  const full = await realpath(resolve(root, path));
  assert(full.startsWith(await realpath(root) + sep));
  return readFile(full);
}
const server = createServer(async (req, res) => {
  const send = (status, data) => res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' }).end(JSON.stringify(data));
  try {
    const path = decodeURIComponent(new URL(req.url, origin).pathname);
    if (path === '/api/auth/session') return send(200, { data: fixture.session });
    if (path === '/api/runtime/bootstrap') return send(200, {data: fixture.session ? {catalog: fixture.catalog, permissions: fixture.session.permissions} : null});
    if (path === '/api/runtime/catalog') return send(200, { data: fixture.catalog });
    if (path === '/api/runtime/components/bridge.js') return res.writeHead(200, { 'content-type': 'text/javascript' }).end(await readFile('sdk/web/host.mjs'));
    if (path === '/api/runtime/frontend/mount') {
      const chunks = []; for await (const chunk of req) chunks.push(chunk);
      const { page_id } = JSON.parse(Buffer.concat(chunks));
      const token = randomUUID().replaceAll('-', '');
      const version = fixture.catalog.page_versions[page_id];
      let parts; try { parts = JSON.parse(version); } catch (_) { parts = abi === 2 ? ['', ...version.split(':')] : [version, version, version]; }
      grants.set(token, { page_id, version, context: fixture.catalog.context }); counts.mount++;
      const asset_sizes = Object.fromEntries(await Promise.all(Object.keys(await assets).map(async name => [name, (await file(frontend, name)).length])));
      return send(200, { data: { abi, token, revision: parts[1], generation: parts[2], session_context: fixture.catalog.session_context, context: fixture.catalog.context, assets: await assets, asset_sizes, src: `/api/runtime/${abi === 2 ? 'components' : 'frontend'}/assets/${token}/index.html` } });
    }
    const route = path.match(/^\/api\/runtime\/(?:frontend|components)\/([^/]+)(?:\/(request|renew))?$/);
    if (route) {
      const [_, token, action] = route;
      if (req.method === 'DELETE') { counts.delete++; grants.delete(token); return res.writeHead(204).end(); }
      const grant = grants.get(token);
      if (!grant || grant.context !== fixture.catalog.context || grant.version !== fixture.catalog.page_versions[grant.page_id]) return send(403, { error: 'Mount revoked' });
      if (action === 'renew') { counts.renew++; return res.writeHead(204).end(); }
      counts.business++;
      const body = JSON.stringify({ items: [], total: 0, completed: 0, tenantId: 'test', userId: 'tester', serverTime: '2026-09-11', storage: 'protocol-fixture' });
      return send(200, { data: abi === 2 ? { status: 200, headers: [], body: Array.from(Buffer.from(body)) } : { status: 200, content_type: 'application/json', body } });
    }
    const asset = path.match(/^\/api\/runtime\/(?:frontend|components)\/assets\/([^/]+)\/(.+)$/);
    let bytes;
    let extension = extname(path);
    const headers = { 'content-type': mime[extension] || 'application/octet-stream', 'cache-control': 'no-store' };
    if (asset) {
      const [_, token, name] = asset;
      if (!grants.has(token)) return send(401, { error: 'Mount missing' });
      counts.asset++;
      bytes = name === '__aio_bridge.js' ? await (abi === 2 ? componentGuest : guest) : name === '__aio_modules.js' ? await modules : await file(frontend, name);
      headers['access-control-allow-origin'] = '*';
      if (name === 'index.html') {
        const document = parse(bytes.toString());
        const head = document.childNodes.find(n => n.tagName === 'html').childNodes.find(n => n.tagName === 'head');
        const element = (tagName, attrs) => ({ nodeName: tagName, tagName, namespaceURI: 'http://www.w3.org/1999/xhtml', attrs: Object.entries(attrs).map(([name, value]) => ({ name, value })), childNodes: [], parentNode: head });
        const normalize = node => {
          if (node.tagName === 'script') for (const attr of node.attrs) if (attr.name === 'type' && ['module', 'importmap'].includes(attr.value)) attr.value += '-shim';
          for (const child of node.childNodes || []) normalize(child);
        };
        normalize(document);
        const prefix = `${origin}/api/runtime/${abi === 2 ? 'components' : 'frontend'}/assets/${token}/`;
        head.childNodes.unshift(element('base', { href: prefix }), element('script', { src: '__aio_bridge.js', 'data-token': token, 'data-root': prefix }), element('script', { src: '__aio_modules.js' }));
        bytes = serialize(document);
        headers['content-security-policy'] = `sandbox allow-scripts; default-src 'none'; script-src ${origin} blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; connect-src blob: ${prefix}; img-src ${origin} data: blob:; font-src ${origin} data:; style-src 'unsafe-inline' blob:; worker-src 'none';`;
      }
    } else {
      if (path === '/favicon.ico') return res.writeHead(204).end();
      bytes = await file(shell, path === '/' ? 'index.html' : path.slice(1));
      if (path === '/') headers['content-type'] = 'text/html';
    }
    res.writeHead(200, headers).end(bytes);
  } catch (error) { send(500, { error: error.message }); }
});

async function run(browser, mobile) {
  reset();
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const errors = [];
  let blockedFallbackFonts = 0;
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() !== 'error') return;
    const text = message.text();
    // 夹具的 Compose 产物未携带中文回退字体，沙箱应继续阻止外部字体下载。
    if (text.includes('https://fonts.gstatic.com/s/notosanssc/') && text.includes('Content Security Policy')) blockedFallbackFonts++;
    else errors.push(text);
  });
  const scene = label => page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: label, exact: true }).click();
  const select = async label => {
    if (mobile) await page.getByRole('button', { name: '打开菜单', exact: true }).click();
    const nav = mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
    await nav.getByRole('button', { name: label, exact: true }).click();
  };
  const refresh = async () => {
    const response = page.waitForResponse(r => r.url().endsWith('/api/runtime/bootstrap'));
    await page.evaluate(() => dispatchEvent(new Event('aio:catalog-invalidated')));
    await response;
  };
  try {
    await page.goto(origin);
    await scene('社区插件');
    const selector = '[data-aio-workspace="test"] iframe[title="Compose 保活"]';
    const iframe = page.locator(selector);
    const frame = page.frameLocator(selector);
    await frame.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 60000 });
    await page.waitForTimeout(400);
    await frame.getByRole('button', { name: 'Counter', exact: true }).click({ force: true });
    await page.mouse.move(0, 0);
    await frame.getByRole('button', { name: '+1', exact: true }).waitFor();
    await page.waitForTimeout(350);
    const canvas = frame.locator('canvas').first();
    const before = PNG.sync.read(await canvas.screenshot());
    const increment = await frame.getByRole('button', { name: '+1', exact: true }).boundingBox();
    assert(increment);
    await page.mouse.click(increment.x + increment.width / 2, increment.y + increment.height / 2);
    await frame.getByText('1', { exact: true }).waitFor();
    const after = PNG.sync.read(await canvas.screenshot());
    let changed = 0; for (let i = 0; i < before.data.length; i += 4) if (before.data.readUInt32BE(i) !== after.data.readUInt32BE(i)) changed++;
    assert(changed > 30);
    const src = await iframe.getAttribute('src');
    const marker = await frame.locator('body').evaluate(() => window.__keepaliveMarker = Math.random());
    await page.evaluate(() => window.__keptFrame = document.querySelector('iframe'));
    const previous = { ...counts };
    await scene('工作区');
    await iframe.waitFor({ state: 'hidden' });
    assert.equal(await frame.locator('body').evaluate(() => window.aioPlugin.visible), false);
    const start = performance.now();
    await scene('社区插件');
    await frame.getByText('1', { exact: true }).waitFor({ timeout: 1000 });
    const warmMs = performance.now() - start;
    assert.equal(await iframe.getAttribute('src'), src);
    assert.equal(await frame.locator('body').evaluate(() => window.__keepaliveMarker), marker);
    assert(await page.evaluate(() => window.__keptFrame === document.querySelector('iframe')));
    assert.equal(counts.mount, previous.mount); assert.equal(counts.delete, previous.delete); assert.equal(counts.asset, previous.asset);
    if (mobile) await page.getByRole('button', { name: '打开菜单', exact: true }).click();
    const nav = mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
    await nav.locator('button[aria-label$="的账户菜单"]').click();
    await page.getByRole('menuitem', { name: '独立账户页', exact: true }).click();
    await page.locator('.application-fullscreen:visible').waitFor();
    await page.getByRole('button', { name: '返回主后台', exact: true }).click();
    await frame.getByText('1', { exact: true }).waitFor();
    assert.equal(await frame.locator('body').evaluate(() => window.__keepaliveMarker), marker);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}.png`) });
    const tenantA = structuredClone(fixture);
    fixture.session.tenant_id = 'tenant-b';
    fixture.catalog.tenant = { id: 'tenant-b', label: 'Tenant B' };
    fixture.catalog.context = 'tenant-b-context';
    await refresh();
    await iframe.waitFor({ state: 'hidden' });
    const oldToken = src.match(/assets\/([^/]+)/)[1];
    await page.waitForFunction(() => document.querySelector('[data-aio-workspace="test"]')?.dataset.aioWorkspaceActive === 'false');
    assert.match(await frame.locator('body').evaluate(() => window.aioPlugin.json('GET', '/tasks').catch(e => e.message)), /暂停/);
    assert.equal((await context.request.post(`${origin}/api/runtime/frontend/${oldToken}/renew`)).status(), 403);
    await scene('社区插件');
    const frameB = page.frameLocator('[data-aio-workspace="tenant-b"] iframe');
    await frameB.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 60000 });
    await frameB.getByRole('button', { name: 'Counter', exact: true }).click({ force: true });
    await frameB.getByRole('button', { name: '+1', exact: true }).waitFor();
    await frameB.getByText('0', { exact: true }).waitFor();
    await frameB.getByRole('button', { name: '+1', exact: true }).click({ force: true });
    await frameB.getByText('1', { exact: true }).waitFor();
    const tenantAssets = counts.asset;
    const remount = page.waitForResponse(r => r.url().endsWith('/frontend/mount'));
    fixture = tenantA;
    await refresh();
    await remount;
    await frame.getByText('1', { exact: true }).waitFor({ timeout: 1000 });
    assert.equal(await iframe.getAttribute('src'), src);
    assert.equal(await frame.locator('body').evaluate(() => window.__keepaliveMarker), marker);
    assert.equal(counts.asset, tenantAssets, 'Returning to A must not reload its iframe or assets');
    assert.equal((await context.request.post(`${origin}/api/runtime/frontend/${oldToken}/renew`)).status(), 403, 'Returning must not revive old grants');
    assert.equal((await frame.locator('body').evaluate(() => window.aioPlugin.json('GET', '/tasks'))).tenantId, 'test');
    fixture.catalog.page_versions['Compose 保活'] = 'version-b';
    await refresh();
    await page.waitForFunction(old => document.querySelector('iframe')?.getAttribute('src') && document.querySelector('iframe').getAttribute('src') !== old, src);
    await frame.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 60000 });
    assert(counts.delete > previous.delete, 'Version replacement must dispose the old frame');
    for (let i = 0; i < 7; i++) await select(`页面 ${i}`);
    await iframe.waitFor({ state: 'detached' });
    await select('Compose 保活');
    await frame.locator('canvas').first().waitFor({ timeout: 60000 });
    fixture.catalog.pages = fixture.catalog.pages.filter(p => p.id !== 'Compose 保活');
    await refresh();
    await iframe.waitFor({ state: 'detached' });
    fixture.catalog.pages.unshift(screen('Compose 保活', { kind: 'frontend', entry: 'index.html' }));
    await refresh();
    await select('Compose 保活');
    await frame.locator('canvas').first().waitFor({ timeout: 60000 });
    const previousContextSrc = await iframe.getAttribute('src');
    await frame.locator('body').evaluate(() => window.__previousContext = true);
    fixture.catalog.context = 'session-b';
    await refresh();
    await page.locator(`iframe[src="${previousContextSrc}"]`).waitFor({ state: 'detached' });
    if (await iframe.count()) {
      await frame.locator('canvas').first().waitFor({ timeout: 60000 });
      assert.notEqual(await iframe.getAttribute('src'), previousContextSrc);
      assert.equal(await frame.locator('body').evaluate(() => window.__previousContext), undefined);
    }
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const downloads = [];
    page.on('request', request => {
      const path = new URL(request.url()).pathname;
      if (/\/frontend\/assets\/.+\.(wasm|mjs|js)$/.test(path) && !/\/(__aio_|import-map-loader|startup)/.test(path)) downloads.push(path);
    });
    await page.reload();
    await scene('社区插件');
    await frame.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 60000 });
    assert.deepEqual(downloads, [], 'Reload must reuse validated module and Wasm bytes');
    const loginSource = await iframe.getAttribute('src');
    fixture.catalog.session_context = 'login-b';
    await refresh();
    await page.locator(`iframe[src="${loginSource}"]`).waitFor({ state: 'detached' });
    await scene('社区插件');
    await frame.locator('canvas').first().waitFor({ timeout: 60000 });
    assert.deepEqual(await page.evaluate(() => caches.keys()), ['aio-plugin-assets-v1-login-b']);
    fixture.session = null;
    const logout = page.waitForResponse(r => r.url().endsWith('/api/runtime/bootstrap'));
    await page.evaluate(() => dispatchEvent(new Event('aio:catalog-invalidated')));
    await logout;
    await iframe.waitFor({ state: 'detached' });
    await page.waitForFunction(async () => (await caches.keys()).length === 0);
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ mobile, blockedFallbackFonts }));
    return { viewport: mobile ? 'mobile' : 'desktop', warmMs, changedCanvasPixels: changed, warmMounts: 0, warmDeletes: 0, warmAssets: 0, retainedState: true, tenantReturnState: true, oldTicketRevoked: true, reloadModuleDownloads: 0, loginCacheIsolation: true, fullscreenReturn: true, versionInvalidation: true, lruEviction: true, permissionRemoval: true, contextInvalidation: true, consoleErrors: 0 };
  } catch (error) {
    await page.screenshot({ path: resolve(output, 'failure.png') });
    console.error(errors); throw error;
  } finally { await context.close(); }
}

module.exports.startFixture = async (protocol = 1) => {
  abi = protocol;
  reset();
  fixture.catalog.page_versions['Compose 保活'] = abi === 2 ? 'v1:v1' : JSON.stringify(['v1', 'v1', 'v1']);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  return { origin, counts, fixture, close: () => new Promise(resolve => server.close(resolve)) };
};

if (require.main === module) (async () => {
  await mkdir(output, { recursive: true });
  await new Promise(resolve => server.listen(Number(process.env.AIO_KEEPALIVE_PREVIEW_PORT || 0), '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  if (process.env.AIO_KEEPALIVE_PREVIEW_PORT) {
    reset();
    console.log(`Isolated Compose navigation fixture: ${origin}`);
    return;
  }
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const report = [await run(browser, false), await run(browser, true)];
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
