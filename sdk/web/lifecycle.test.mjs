import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';
import { test } from 'node:test';

function fixture() {
  const listeners = [];
  const sent = [];
  const timers = [];
  const parent = { postMessage: value => sent.push(value) };
  const window = { parent, addEventListener: (kind, fn) => { if (kind === 'message') listeners.push(fn); } };
  const context = vm.createContext({ window, parent, document: { currentScript: { dataset: { token: 't' } }, body: { querySelector: () => ({}) } },
    crypto: webcrypto, URL, Uint8Array, TextEncoder, TextDecoder, CustomEvent: class {}, dispatchEvent() {},
    addEventListener: window.addEventListener, setInterval: () => 1, clearInterval() {},
    setTimeout: fn => { timers.push(fn); return timers.length; }, clearTimeout() {} });
  vm.runInContext(readFileSync(new URL('./lifecycle.js', import.meta.url), 'utf8'), context);
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), context);
  const receive = (data, source = parent) => listeners.forEach(fn => fn({ data, source }));
  const state = (data = {}, source = parent) => receive({ channel: 'aio-lifecycle', token: 't', kind: 'state', ...data }, source);
  return { window, sent, timers, state, receive };
}

test('preparation delays business and its timeout until authenticated activation', async () => {
  const f = fixture();
  const request = f.window.aioPlugin.json('GET', '/graph');
  assert.equal(f.sent.length, 1);
  assert.equal(f.timers.length, 0);
  f.state({ active: true }, {});
  f.state({ active: false });
  await Promise.resolve();
  assert.equal(f.sent.length, 1);
  f.state({ active: true });
  await Promise.resolve();
  assert.equal(f.sent.length, 2);
  assert.equal(f.timers.length, 1);
  const message = f.sent[1];
  f.receive({ protocol: 'aio:plugin@2', kind: 'response', id: message.id, response: { status: 200, headers: [], body: [] } });
  assert.equal(await request, null);
});

test('tenant suspension rejects preparation without sending any service request', async () => {
  const f = fixture();
  const waiting = f.window.aioPlugin.json('POST', '/write', {});
  f.state({ suspended: true });
  await assert.rejects(waiting, /暂停/);
  await assert.rejects(f.window.aioPlugin.json('GET', '/graph'), /暂停/);
  assert.equal(f.sent.length, 1);
});

test('prepared request queue is bounded and switching away preserves activation', async () => {
  const f = fixture();
  const pending = Array.from({ length: 16 }, () => f.window.aioLifecycle.whenActive());
  await assert.rejects(f.window.aioLifecycle.whenActive(), /限制/);
  f.state({ active: true });
  await Promise.all(pending);
  f.state({ active: false, visible: false });
  await f.window.aioLifecycle.whenActive();
  assert.equal(f.window.aioLifecycle.activated, true);
});
