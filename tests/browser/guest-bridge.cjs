const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { test } = require('node:test');

test('JSON bridge preserves HTTP errors, payloads and empty responses', async () => {
  let receive;
  let outgoing;
  const parent = { postMessage(value) { outgoing = value; } };
  const window = { fetch() {} };
  vm.runInNewContext(fs.readFileSync('lib/plugin/host/src/runtime/server/frontend_guest.js', 'utf8'), {
    document: { currentScript: { dataset: { token: 'mount' }, src: 'https://aio.test/api/runtime/frontend/assets/mount/__aio_bridge.js' } },
    URL, Request, Response, MutationObserver: class { observe() {} },
    window, parent, setTimeout, clearTimeout,
    addEventListener(type, callback) { if (type === 'message') receive = callback; },
  });
  const reply = response => receive({ source: parent, data: { channel: 'aio-plugin', token: 'mount', id: outgoing.id, response } });
  const create = window.aioPlugin.json('POST', '/tasks', { title: 'Persist me' });
  assert.equal(outgoing.request.body, '{"title":"Persist me"}');
  reply({ status: 201, body: '{"id":4}' });
  assert.equal((await create).id, 4);
  const remove = window.aioPlugin.json('DELETE', '/tasks/4');
  reply({ status: 204, body: '' });
  assert.equal(await remove, null);
  const invalid = window.aioPlugin.json('POST', '/tasks', { title: '' });
  reply({ status: 400, body: '{"error":"Title required"}' });
  await assert.rejects(invalid, /Title required/);
});

test('visibility events are scoped to the parent and mount and can unsubscribe', () => {
  let receive;
  const parent = {};
  const window = { fetch() {} };
  vm.runInNewContext(fs.readFileSync('lib/plugin/host/src/runtime/server/frontend_guest.js', 'utf8'), {
    document: { currentScript: { dataset: { token: 'mount' }, src: 'https://aio.test/api/runtime/frontend/assets/mount/__aio_bridge.js' } },
    URL, Request, Response, MutationObserver: class { observe() {} },
    window, parent, setTimeout, clearTimeout,
    addEventListener(type, callback) { if (type === 'message') receive = callback; },
  });
  const observed = [];
  const stop = window.aioPlugin.onVisibilityChange(value => observed.push(value));
  const event = { source: parent, data: { channel: 'aio-plugin', token: 'mount', lifecycle: 'visibility', visible: false } };
  receive({ ...event, source: {} });
  receive({ ...event, data: { ...event.data, token: 'other' } });
  assert.equal(window.aioPlugin.visible, true);
  receive(event);
  assert.equal(window.aioPlugin.visible, false);
  assert.deepEqual(observed, [true, false]);
  stop();
  receive({ ...event, data: { ...event.data, visible: true } });
  assert.deepEqual(observed, [true, false]);
});
