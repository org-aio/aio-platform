const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { runInNewContext } = require('node:vm');

function fixture() {
  const calls = [], loaded = [], options = [];
  const mount = { abi: 2, token: 'warm', revision: 'r', generation: 'g', session_context: 'session', context: 'tenant', assets: { 'index.html': 'h', 'app.mjs': 'a', 'app.wasm': 'b', 'app.wasm.map': 'c' }, asset_sizes: { 'index.html': 10, 'app.mjs': 10, 'app.wasm': 100, 'app.wasm.map': 20 } };
  const config = { session_context: 'session', context: 'tenant', pages: [{ id: 'one', version: 'r:g' }, { id: 'two', version: 'r:g' }] };
  const activePages = [];
  const env = {
    caches: {}, navigator: {}, document: {
      visibilityState: 'visible',
      querySelectorAll: selector => selector === '[data-aio-page-active="true"]' ? activePages : [],
    }, AbortSignal,
    setTimeout: callback => setTimeout(callback, 0),
    fetch: async (url, request) => { calls.push({ url, request }); return { ok: true, status: 200, json: async () => ({ data: mount }) }; },
    createFrontendAssetCache: config => { options.push(config); return async (path, ticket, signal) => { signal.throwIfAborted(); loaded.push({ path, ticket }); }; },
  };
  const warm = runInNewContext(readFileSync('lib/plugin/host/src/runtime/frontend_preload.js', 'utf8') + '\nwarmFrontendAssets', env);
  return { warm, env, config, mount, loaded, calls, options, activePages };
}

test('warming only reads package assets, coalesces revisions and releases every grant', async () => {
  const f = fixture();
  await f.warm(f.config, new AbortController().signal);
  assert.deepEqual(f.loaded.map(item => item.path), ['app.mjs', 'app.wasm']);
  assert.equal(f.options.length, 1);
  assert.equal(f.options[0].background, true);
  assert.equal(f.calls.filter(call => call.request.method === 'DELETE').length, 2);
  assert(f.calls.every(call => call.url.startsWith('/api/runtime/frontend/')));
});

test('stale login, tenant and version never warm and always release the returned grant', async () => {
  for (const change of [{ session_context: 'another' }, { context: 'another' }, { generation: 'new' }]) {
    const f = fixture();
    Object.assign(f.mount, change);
    await f.warm(f.config, new AbortController().signal);
    assert.equal(f.loaded.length, 0);
    assert.equal(f.calls.length, 2);
    assert.equal(f.calls[1].request.method, 'DELETE');
  }
});

test('cancellation stops queued resources and revokes its temporary grant', async () => {
  const f = fixture();
  const controller = new AbortController();
  f.env.createFrontendAssetCache = () => async () => controller.abort();
  await assert.rejects(f.warm(f.config, controller.signal));
  assert.equal(f.calls.filter(call => call.request.method === 'POST').length, 1);
  assert.equal(f.calls.at(-1).request.method, 'DELETE');
});

test('background admission respects byte budget, item budget and data saver', async () => {
  const f = fixture();
  f.mount.asset_sizes['app.wasm'] = 81 * 1024 * 1024;
  await f.warm(f.config, new AbortController().signal);
  assert.deepEqual(f.loaded.map(item => item.path), ['app.mjs']);
  for (const connection of [{ saveData: true }, { effectiveType: '2g' }]) {
    const f = fixture(); f.env.navigator.connection = connection;
    await f.warm(f.config, new AbortController().signal);
    assert.equal(f.calls.length, 0);
  }
});

test('legacy catalog versions and shared resources use the same bounded pipeline', async () => {
  const f = fixture();
  f.mount.abi = null;
  f.config.pages.forEach(page => { page.version = JSON.stringify(['source', 'r', 'g']); });
  await f.warm(f.config, new AbortController().signal);
  assert.deepEqual(f.loaded.map(item => item.path), ['app.mjs', 'app.wasm']);
  assert.equal(f.calls.filter(call => call.request.method === 'DELETE').length, 2);
});

test('only completely warmed releases can create a background instance', async () => {
  const f = fixture();
  const prepared = [];
  await f.warm(f.config, new AbortController().signal, async id => prepared.push(id));
  assert.deepEqual(prepared, ['one']);
  const large = fixture();
  large.mount.asset_sizes['app.wasm'] = 81 * 1024 * 1024;
  await large.warm(large.config, new AbortController().signal, async () => assert.fail('Budget bypassed by instance preparation'));
});

test('a stalled preparation stops further instance creation but still warms resources', async () => {
  const f = fixture();
  f.config.pages = ['one', 'two', 'three'].map(id => ({ id, version: `${id}:g` }));
  f.env.fetch = async (url, request) => {
    f.calls.push({ url, request });
    const id = request.body ? JSON.parse(request.body).page_id : '';
    return { ok: true, status: 200, json: async () => ({ data: { ...f.mount, revision: id, token: id } }) };
  };
  const prepared = [];
  await f.warm(f.config, new AbortController().signal, async id => { prepared.push(id); return false; });
  assert.deepEqual(prepared, ['one']);
  assert.equal(f.calls.filter(call => call.request.method === 'DELETE').length, 3);
});

test('the active page is never warmed in the background', async () => {
  const f = fixture();
  f.activePages.push({ dataset: { aioPage: 'one' } });
  await f.warm(f.config, new AbortController().signal);
  assert.deepEqual(f.calls.filter(call => call.request.method === 'POST').map(call => JSON.parse(call.request.body).page_id), ['two']);
  assert.deepEqual(f.loaded.map(item => item.path), ['app.mjs', 'app.wasm']);
});
