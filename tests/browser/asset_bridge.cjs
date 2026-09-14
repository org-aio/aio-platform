const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { runInNewContext } = require('node:vm');

test('asset bridge scopes callers, guards paused pages and transfers independent buffers', async () => {
  let receive, active = true;
  const bytes = new Uint8Array([1, 2, 3]).buffer;
  const calls = [], replies = [];
  const child = { postMessage: (reply, origin, transfers) => replies.push({ reply, origin, transfers }) };
  const create = runInNewContext(readFileSync('lib/plugin/host/src/runtime/frontend_assets.js', 'utf8') + '\nmountFrontendAssets', {
    AbortController,
    window: { addEventListener: (_, handler) => { receive = handler; }, removeEventListener: (_, handler) => assert.equal(handler, receive) },
    createFrontendAssetCache: () => async (path, token) => { calls.push({ path, token }); return { bytes, type: 'application/wasm' }; },
  });
  const bridge = create({ contentWindow: child }, { token: 'current' }, () => active);
  const event = { source: child, origin: 'null', data: { channel: 'aio-assets', token: 'current', id: '1', path: 'app.wasm' } };
  await receive({ ...event, source: {} });
  await receive({ ...event, origin: 'https://other.test' });
  await receive({ ...event, data: { ...event.data, token: 'other' } });
  assert.equal(calls.length, 0);
  await receive(event);
  assert.equal(calls.length, 1);
  assert.notEqual(replies[0].reply.bytes, bytes);
  assert.deepEqual(new Uint8Array(replies[0].reply.bytes), new Uint8Array(bytes));
  assert.equal(replies[0].transfers[0], replies[0].reply.bytes);
  active = false;
  await receive({ ...event, data: { ...event.data, id: '2' } });
  assert.match(replies[1].reply.error, /暂停/);
  assert.equal(calls.length, 1);
  bridge.dispose();
});
