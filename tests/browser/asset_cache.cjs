const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { createHash, webcrypto } = require('node:crypto');
const { runInNewContext } = require('node:vm');

function fixture() {
  const stores = new Map();
  const requests = [];
  const bytes = Buffer.from('export const value = 42;');
  const sha = createHash('sha256').update(bytes).digest('hex');
  const config = { revision: 'revision-a', session_context: 'login-a', assets: { 'app.js': sha } };
  const caches = {
    keys: async () => [...stores.keys()],
    delete: async name => stores.delete(name),
    open: async name => {
      if (stores.has(name)) return stores.get(name);
      const entries = new Map();
      const cache = {
        entries,
        keys: async () => [...entries.keys()],
        match: async key => entries.get(key)?.clone(),
        delete: async key => entries.delete(key),
        put: async (key, response) => entries.set(key, response.clone()),
      };
      stores.set(name, cache);
      return cache;
    },
  };
  const env = {
    window: {}, caches, crypto: webcrypto, Response, URL, AbortSignal, location: { origin: 'https://aio.test' },
    setTimeout: callback => setTimeout(callback, 0),
    fetch: async (url, options) => {
      requests.push({ url, options });
      return new Response(bytes, { headers: { 'content-type': 'text/javascript' } });
    },
  };
  const create = runInNewContext(readFileSync('lib/plugin/host/src/runtime/frontend_cache.js', 'utf8') + '\ncreateFrontendAssetCache', env);
  return { create, config, caches, stores, requests, bytes, env };
}

test('same-session instances and recreated hosts reuse verified immutable bytes, never business responses', async () => {
  const f = fixture();
  const first = f.create(f.config);
  const a = await first('app.js', 'ticket-a');
  const b = await f.create(f.config)('app.js', 'new-ticket');
  assert.deepEqual(Buffer.from(a.bytes), f.bytes);
  assert.deepEqual(Buffer.from(b.bytes), f.bytes);
  assert.equal(f.requests.length, 1);
  assert.equal(f.requests[0].options.credentials, 'same-origin');
  assert.equal(f.requests[0].options.redirect, 'error');
  await assert.rejects(first('/tasks', 'ticket-a'), /未声明/);
  await assert.rejects(first('../app.js', 'ticket-a'), /未声明/);
  assert.equal(f.requests.length, 1);
});

test('damaged cache is replaced and damaged network artifacts are never admitted', async () => {
  const f = fixture();
  const load = f.create(f.config);
  await load('app.js', 'ticket');
  const cache = f.stores.get('aio-plugin-assets-v1-login-a');
  const [key] = await cache.keys();
  await cache.put(key, new Response('tampered'));
  await load('app.js', 'ticket');
  assert.equal(f.requests.length, 2);
  assert.equal(await (await cache.match(key)).text(), f.bytes.toString());
  await cache.delete(key);
  f.env.fetch = async () => new Response('wrong digest');
  await assert.rejects(load('app.js', 'ticket'), /摘要/);
  assert.equal((await cache.keys()).length, 0);
});

test('concurrent mounts coalesce downloads and login changes remove the old namespace', async () => {
  const f = fixture();
  await Promise.all([f.create(f.config)('app.js', 'one'), f.create(f.config)('app.js', 'two')]);
  assert.equal(f.requests.length, 1);
  await f.create({ ...f.config, session_context: 'login-b' })('app.js', 'three');
  assert.deepEqual(await f.caches.keys(), ['aio-plugin-assets-v1-login-b']);
  assert.equal(f.requests.length, 2);
  const keys = await f.stores.get('aio-plugin-assets-v1-login-b').keys();
  assert(!keys[0].includes('three'), 'Cache keys must not contain authorization tickets');
});

test('cache denial falls back to verified downloads', async () => {
  const f = fixture();
  f.caches.open = async () => { throw new Error('disabled'); };
  assert.deepEqual(Buffer.from((await f.create(f.config)('app.js', 'ticket')).bytes), f.bytes);
  assert.equal(f.requests.length, 1);
});

test('writes from different plugin mounts obey the shared entry quota', async () => {
  const f = fixture();
  const other = Buffer.from('export const value = 43;');
  const otherSha = createHash('sha256').update(other).digest('hex');
  f.env.fetch = async url => new Response(url.includes('/two/') ? other : f.bytes);
  const cache = await f.caches.open('aio-plugin-assets-v1-login-a');
  for (let i = 0; i < 512; i++) await cache.put(`old-${i}`, new Response('x', { headers: { 'content-length': '1' } }));
  await Promise.all([
    f.create(f.config)('app.js', 'one'),
    f.create({ ...f.config, revision: 'revision-b', assets: { 'app.js': otherSha } })('app.js', 'two'),
  ]);
  const keys = await cache.keys();
  assert.equal(keys.length, 512);
  assert(keys.some(key => key.includes(f.config.assets['app.js'])));
  assert(keys.some(key => key.includes(otherSha)));
  assert(!keys.includes('old-0'));
  assert(!keys.includes('old-1'));
});

test('large asset transfers have a separate timeout from business requests', () => {
  const timeouts = [];
  const window = { fetch: () => {} };
  const document = { currentScript: { dataset: { token: 'channel' }, src: 'https://aio.test/api/runtime/frontend/assets/channel/__aio_bridge.js' }, baseURI: 'https://aio.test/api/runtime/frontend/assets/channel/' };
  runInNewContext(readFileSync('lib/plugin/host/src/runtime/server/frontend_guest.js', 'utf8'), {
    window, document, parent: { postMessage() {} }, URL, Request, Response,
    addEventListener() {}, MutationObserver: class { observe() {} },
    setTimeout: (_, ms) => timeouts.push(ms), clearTimeout() {},
  });
  void window.aioPlugin.request({ path: '/tasks' });
  void window.fetch('frontend.wasm');
  assert.deepEqual(timeouts, [30000, 600000]);
});

test('transient transport and server failures retry before verified delivery', async () => {
  const f = fixture();
  let calls = 0;
  f.env.fetch = async () => {
    calls++;
    if (calls === 1) throw new TypeError('network');
    if (calls === 2) return new Response('', { status: 503 });
    return new Response(f.bytes);
  };
  const result = await f.create(f.config)('app.js', 'ticket');
  assert.equal(calls, 3);
  assert.deepEqual(Buffer.from(result.bytes), f.bytes);
});

test('persistent download failures stop after three attempts', async () => {
  const f = fixture();
  let calls = 0;
  f.env.fetch = async () => { calls++; return new Response('', { status: 503 }); };
  await assert.rejects(f.create(f.config)('app.js', 'ticket'), /HTTP 503/);
  assert.equal(calls, 3);
});

test('revoked access and integrity mismatches do not retry', async () => {
  for (const [response, error] of [[new Response('', { status: 403 }), /HTTP 403/], [new Response('modified'), /摘要/]]) {
    const f = fixture();
    let calls = 0;
    f.env.fetch = async () => { calls++; return response; };
    await assert.rejects(f.create(f.config)('app.js', 'ticket'), error);
    assert.equal(calls, 1);
  }
});

test('disposed mounts do not continue retrying', async () => {
  const f = fixture();
  const controller = new AbortController();
  let calls = 0;
  f.env.fetch = async () => { calls++; controller.abort(); throw new Error('cancelled'); };
  await assert.rejects(f.create(f.config)('app.js', 'ticket', controller.signal), /cancelled/);
  assert.equal(calls, 1);
});

test('v2 warming and foreground mounts share verified bytes across distinct tickets', async () => {
  const f = fixture();
  const config = { ...f.config, abi: 2 };
  await f.create({ ...config, background: true })('app.js', 'warm');
  await f.create(config)('app.js', 'foreground');
  await f.create({ ...config, revision: 'other-plugin' })('app.js', 'other-plugin');
  await f.create({ ...config, assets: { 'renamed.js': config.assets['app.js'] } })('renamed.js', 'renamed');
  assert.equal(f.requests.length, 1);
  assert.equal(f.requests[0].url, '/api/runtime/components/assets/warm/app.js');
  assert.equal(f.requests[0].options.priority, 'low');
});

test('foreground takes over an aborted background transfer with its own grant', async () => {
  const f = fixture();
  const controller = new AbortController();
  let started;
  const ready = new Promise(resolve => { started = resolve; });
  f.env.fetch = async (url, options) => {
    f.requests.push({ url, options });
    if (url.includes('/warm/')) {
      started();
      return new Promise((_, reject) => options.signal.addEventListener('abort', () => reject(new Error('cancelled')), { once: true }));
    }
    return new Response(f.bytes);
  };
  const warm = f.create({ ...f.config, abi: 2, background: true })('app.js', 'warm', controller.signal);
  const rejected = assert.rejects(warm, /cancelled/);
  await ready;
  const foreground = f.create({ ...f.config, abi: 2 })('app.js', 'foreground');
  controller.abort();
  await rejected;
  assert.deepEqual(Buffer.from((await foreground).bytes), f.bytes);
  assert.equal(f.requests.length, 2);
  assert(f.requests[1].url.includes('/foreground/'));
});

test('foreground immediately replaces an in-flight background download even before abort propagation', async () => {
  const f = fixture();
  const controller = new AbortController();
  let releaseWarm;
  const warmStarted = new Promise(resolve => { releaseWarm = resolve; });
  f.env.fetch = async (url, options) => {
    f.requests.push({ url, options });
    if (url.includes('/warm/')) {
      releaseWarm();
      return new Promise((_, reject) => options.signal.addEventListener('abort', () => reject(new Error('cancelled')), { once: true }));
    }
    return new Response(f.bytes);
  };
  const warm = f.create({ ...f.config, abi: 2, background: true })('app.js', 'warm', controller.signal);
  await warmStarted;
  const foreground = f.create({ ...f.config, abi: 2 })('app.js', 'foreground');
  assert.deepEqual(Buffer.from((await foreground).bytes), f.bytes);
  assert.equal(f.requests.length, 2);
  assert(f.requests[1].url.includes('/foreground/'));
  controller.abort();
  await assert.rejects(warm, /cancelled/);
});

test('cancelled readers cannot receive a cached resource', async () => {
  const f = fixture();
  await f.create(f.config)('app.js', 'ticket');
  await assert.rejects(f.create(f.config)('app.js', 'ticket', AbortSignal.abort()));
  assert.equal(f.requests.length, 1);
});
