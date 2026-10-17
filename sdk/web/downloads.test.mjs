import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import {webcrypto} from 'node:crypto';
import {mountBridge} from './host.mjs';

test('流式下载只传插件路径，宿主即时核验手势、来源和可见性', async () => {
  let listener, reply, received, sent, calls = 0;
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const activation = {isActive: false};
  globalThis.window = {addEventListener: (_, fn) => {listener = fn;}, removeEventListener: () => {}};
  globalThis.document = {hasFocus: () => true, hidden: false};
  Object.defineProperty(globalThis, 'navigator', {configurable: true, value: {userActivation: activation}});
  const child = {postMessage: message => {reply = message;}};
  const parent = {postMessage: message => {sent = message;}};
  const guest = {parent, addEventListener: (_, fn) => {received = fn;}};
  const frame = {contentWindow: child, checkVisibility: () => true};
  vm.runInNewContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), {
    window: guest, document: {currentScript: {dataset: {}}}, crypto: webcrypto,
    Uint8Array, TextEncoder, TextDecoder, URL, setTimeout, clearTimeout,
  });
  let dispose = () => {};
  try {
    for (const path of ['https://outside.test/a', '//outside/a', '/a/../b', '/a/%2e', '/a?secret=x', '/a\\b', '/a//b']) {
      await assert.rejects(guest.aioPlugin.downloadFrom(path, 'file.bin'));
    }
    const waiting = guest.aioPlugin.downloadFrom('/files/id', 'large.bin', 'version=2');
    assert.equal(sent.kind, 'download-request');
    assert.equal(sent.body, undefined);
    const event = {source: child, origin: 'null', data: sent};
    dispose = mountBridge(frame, () => assert.fail('下载进入 JSON 执行器'));
    activation.isActive = true;
    await listener(event); assert.match(reply.error, /denied/); dispose();
    dispose = mountBridge(frame, () => assert.fail('下载进入 JSON 执行器'), {
      downloadRequest: value => {calls++; assert.equal(activation.isActive, true); assert.equal(value.query, 'version=2'); return {status: 204, headers: [], body: []};},
    });
    activation.isActive = false;
    await listener(event); assert.match(reply.error, /denied/);
    activation.isActive = true;
    await listener({...event, origin: 'https://foreign.test'});
    await listener({...event, source: {}});
    document.hidden = true; await listener(event); assert.match(reply.error, /denied/);
    document.hidden = false;
    frame.checkVisibility = () => false; await listener(event); assert.match(reply.error, /denied/);
    frame.checkVisibility = () => true;
    for (const override of [{path:'//outside/a'}, {path:'/a/../b'}, {name:'../secret'}, {query:'a\nCookie:x'}]) {
      await listener({...event, data:{...sent,...override}}); assert.match(reply.error, /denied/);
    }
    assert.equal(calls, 0);
    const delivered = listener(event);
    assert.equal(calls, 1);
    await delivered;
    received({source: parent, data: reply});
    assert.equal((await waiting).status, 204);
    dispose(); await listener(event); assert.equal(calls, 1);
  } finally {
    dispose(); delete globalThis.window; delete globalThis.document;
    if (previous) Object.defineProperty(globalThis, 'navigator', previous); else delete globalThis.navigator;
  }
});
