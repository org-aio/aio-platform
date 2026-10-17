import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import { webcrypto } from 'node:crypto';
import { mountBridge } from './host.mjs';

function guestContext(window) {
  return vm.createContext({
    window, document: { currentScript: { dataset: {} } },
    crypto: webcrypto, Uint8Array, TextEncoder, TextDecoder, URL, setTimeout, clearTimeout,
  });
}

test('管理调用使用独立入口并遵守来源、卸载及默认拒绝边界', async () => {
  let listener, reply, received, sent, calls = 0;
  globalThis.window = { addEventListener: (_, fn) => { listener = fn; }, removeEventListener: () => {} };
  const child = { postMessage: message => { reply = message; } };
  const parent = { postMessage: message => { sent = message; } };
  const guest = { parent, addEventListener: (_, fn) => { received = fn; } };
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), guestContext(guest));
  const waiting = guest.aioPlugin.manage({ path: '/api/runtime/catalog?view=installed' });
  assert.equal(sent.kind, 'manage');
  assert.equal(sent.request.query, 'view=installed');
  const event = { source: child, origin: 'null', data: sent };
  try {
    let unmount = mountBridge({ contentWindow: child }, () => assert.fail('管理调用进入业务执行器'));
    await listener(event);
    assert.match(reply.error, /Management access unavailable/);
    unmount();
    unmount = mountBridge({ contentWindow: child }, () => assert.fail('管理调用进入业务执行器'), {
      manage: async input => { calls++; assert.equal(input.path, '/api/runtime/catalog'); return { status: 200, headers: [], body: [] }; },
    });
    await listener({ ...event, source: {} });
    await listener({ ...event, origin: 'https://outside.test' });
    assert.equal(calls, 0);
    await listener(event);
    assert.equal(calls, 1);
    received({ source: parent, data: reply });
    assert.equal((await waiting).status, 200);
    unmount();
    await listener(event);
    assert.equal(calls, 1);
  } finally { delete globalThis.window; }
});

test('clipboard requires a host grant and an active gesture', async () => {
  let listener, reply, copied;
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  globalThis.window = {addEventListener: (_,fn)=>{listener=fn;},removeEventListener:()=>{}};
  globalThis.document = {hasFocus:()=>true};
  const activation={isActive:false};
  Object.defineProperty(globalThis,'navigator',{configurable:true,value:{userActivation:activation,clipboard:{writeText:async text=>{copied=text;}}}});
  const child={postMessage:message=>{reply=message;}};
  const event={source:child,origin:'null',data:{protocol:'aio:plugin@2',kind:'clipboard',id:'copy',text:'test-only-secret'}};
  try {
    const denied=mountBridge({contentWindow:child},()=>assert.fail('clipboard reached service'));
    activation.isActive=true;
    await listener(event); assert(reply.error); assert.equal(copied,undefined); denied();
    const allowed=mountBridge({contentWindow:child},()=>assert.fail('clipboard reached service'),{clipboard:true});
    activation.isActive=false;
    await listener(event); assert(reply.error); assert.equal(copied,undefined);
    activation.isActive=true;
    await listener(event); assert.equal(reply.response.status,204); assert.equal(copied,'test-only-secret');
    allowed();
  } finally {
    delete globalThis.window;delete globalThis.document;
    if(previous) Object.defineProperty(globalThis,'navigator',previous); else delete globalThis.navigator;
  }
});

test('guest transports binary bodies and ignores other windows', async () => {
  let receive;
  let sent;
  const parent = { postMessage: message => { sent = message; } };
  const window = { parent, addEventListener: (_kind, listener) => { receive = listener; } };
  const context = guestContext(window);
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), context);
  const result = window.aioPlugin.request({ path: '/file', method: 'POST', body: new Uint8Array([0, 255, 128]) });
  assert.deepEqual([...sent.request.body], [0, 255, 128]);
  receive({ source: {}, data: { protocol: 'aio:plugin@2', kind: 'response', id: sent.id, error: 'spoof' } });
  receive({ source: parent, data: { protocol: 'aio:plugin@2', kind: 'response', id: sent.id, response: { status: 200, headers: [], body: [0, 255, 128] } } });
  assert.deepEqual([...(await result).body], [0, 255, 128]);
  await assert.rejects(window.aioPlugin.request({ path: '//outside', body: new Uint8Array() }));
});

test('guest creates request ids in sandboxed opaque origins without randomUUID', async () => {
  let receive;
  let sent;
  const parent = { postMessage: message => { sent = message; } };
  const window = { parent, addEventListener: (_kind, listener) => { receive = listener; } };
  const context = vm.createContext({
    window, document: { currentScript: { dataset: {} } },
    Uint8Array, TextEncoder, TextDecoder, URL, setTimeout, clearTimeout,
  });
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), context);
  const request = window.aioPlugin.request({ path: '/file', body: new Uint8Array() });
  assert.match(sent.id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  receive({ source: parent, data: { protocol: 'aio:plugin@2', kind: 'response', id: sent.id, response: { status: 204, headers: [], body: [] } } });
  assert.equal((await request).status, 204);
});

test('host checks opaque origin, frame ownership and revocation', async () => {
  let listener;
  let reply;
  let calls = 0;
  let finish;
  globalThis.window = { addEventListener: (_kind, fn) => { listener = fn; }, removeEventListener: (_kind, fn) => { assert.equal(fn, listener); } };
  const child = { postMessage: message => { reply = message; } };
  const unmount = mountBridge({ contentWindow: child }, async () => { calls++; return new Promise(resolve => { finish = resolve; }); });
  const data = { protocol: 'aio:plugin@2', kind: 'request', id: 'one', request: { path: '/file', body: new Uint8Array([255]) } };
  await listener({ source: {}, origin: 'null', data });
  await listener({ source: child, origin: 'https://outside.example', data });
  assert.equal(calls, 0);
  const pending = listener({ source: child, origin: 'null', data });
  assert.equal(calls, 1);
  unmount();
  finish({ status: 200, body: [255], headers: [] });
  await pending;
  assert.equal(reply, undefined);
  delete globalThis.window;
});

test('guest JSON handles empty success, structured content and HTTP failure', async () => {
  let receive, sent;
  const parent = { postMessage: message => { sent = message; } };
  const window = { parent, addEventListener: (_kind, listener) => { receive = listener; } };
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), guestContext(window));
  const reply = (status, body) => receive({ source: parent, data: {
    protocol: 'aio:plugin@2', kind: 'response', id: sent.id,
    response: { status, headers: [], body: new TextEncoder().encode(body) },
  } });
  const empty = window.aioPlugin.json('DELETE', '/tasks/1');
  reply(204, '');
  assert.equal(await empty, null);
  const json = window.aioPlugin.json('POST', '/tasks', { title: 'test' });
  assert.equal(new TextDecoder().decode(sent.request.body), '{"title":"test"}');
  reply(201, '{"id":1}');
  assert.equal((await json).id, 1);
  const failed = window.aioPlugin.json('GET', '/tasks/2');
  reply(404, '{"error":"Missing task"}');
  await assert.rejects(failed, /Missing task/);
});

test('guest splits service URLs into the v2 path and query fields', async () => {
  let receive, sent;
  const parent = {postMessage: message => {sent = message;}};
  const window = {parent, addEventListener: (_, listener) => {receive = listener;}};
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), guestContext(window));
  for (const input of [
    {path: '/graph?spaceId=personal&alias=a%2Bb'},
    {path: '/graph', query: 'spaceId=personal&alias=a%2Bb'},
  ]) {
    const result = window.aioPlugin.request(input);
    assert.equal(sent.request.path, '/graph');
    assert.equal(sent.request.query, 'spaceId=personal&alias=a%2Bb');
    receive({source:parent, data:{protocol:'aio:plugin@2',kind:'response',id:sent.id,response:{status:200,headers:[],body:[]}}});
    await result;
  }
  for (const path of ['https://outside.test/graph', '//outside.test/graph', '/\\outside.test/graph', '/graph#fragment']) {
    await assert.rejects(window.aioPlugin.request({path}), /Invalid service path/);
  }
  await assert.rejects(window.aioPlugin.request({path:'/graph?spaceId=one',query:'spaceId=two'}), /Specify query only once/);
});

test('download bridge requires a visible owned frame and active gesture, and preserves binary content', async () => {
  let listener, reply, clicked, blob;
  const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const originalURL = globalThis.URL;
  const originalTimeout = globalThis.setTimeout;
  const activation = { isActive: false };
  const child = { postMessage: (message) => { reply = message; } };
  const frame = { contentWindow: child, checkVisibility: () => true };
  const link = { click: () => { clicked = { name: link.download, href: link.href }; }, remove: () => {} };
  globalThis.window = { addEventListener: (_, callback) => { listener = callback; }, removeEventListener: () => {} };
  globalThis.document = { hasFocus: () => true, createElement: () => link, body: { append: () => {} } };
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { userActivation: activation } });
  globalThis.URL = { createObjectURL: (value) => { blob = value; return 'blob:host-file'; }, revokeObjectURL: () => {} };
  globalThis.setTimeout = () => 0;
  const event = { source: child, origin: 'null', data: { protocol: 'aio:plugin@2', kind: 'download', id: 'download', name: 'note.md', mime: 'text/markdown', body: new Uint8Array([0, 255, 128]) } };
  const dispose = mountBridge(frame, () => assert.fail('download must not reach the service'));
  try {
    await listener(event);
    assert.match(reply.error, /denied/);
    assert.equal(clicked, undefined);
    activation.isActive = true;
    await listener({ ...event, source: {} });
    assert.equal(clicked, undefined);
    frame.checkVisibility = () => false;
    await listener(event);
    assert.match(reply.error, /denied/);
    frame.checkVisibility = () => true;
    for (const name of ['../note.md', 'bad\\name.md', 'bad\nname.md']) {
      await listener({ ...event, data: { ...event.data, name } });
      assert.match(reply.error, /denied/);
    }
    await listener({ ...event, data: { ...event.data, body: new Uint8Array(16 * 1024 * 1024 + 1) } });
    assert.match(reply.error, /denied/);
    await listener(event);
    assert.equal(reply.response.status, 204);
    assert.equal(clicked.name, 'note.md');
    assert.deepEqual([...new Uint8Array(await blob.arrayBuffer())], [0, 255, 128]);
    dispose();
    const deny = mountBridge(frame, () => {}, { download: false });
    await listener(event);
    assert.match(reply.error, /denied/);
    deny();
  } finally {
    dispose();
    delete globalThis.window;
    delete globalThis.document;
    if (navigatorDescriptor) Object.defineProperty(globalThis, 'navigator', navigatorDescriptor); else delete globalThis.navigator;
    globalThis.URL = originalURL;
    globalThis.setTimeout = originalTimeout;
  }
});

test('guest download sends binary data and validates filenames before transport', async () => {
  let receive, sent;
  const parent = { postMessage: (message) => { sent = message; } };
  const window = { parent, addEventListener: (_, callback) => { receive = callback; } };
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), guestContext(window));
  await assert.rejects(window.aioPlugin.download('../bad.md', new Uint8Array()), /Invalid download/);
  const pending = window.aioPlugin.download('note.md', new Uint8Array([0, 255]), 'text/markdown');
  assert.equal(sent.kind, 'download');
  assert.deepEqual([...sent.body], [0, 255]);
  receive({ source: parent, data: { protocol: 'aio:plugin@2', kind: 'response', id: sent.id, response: { status: 204, headers: [], body: [] } } });
  assert.equal((await pending).status, 204);
});

test('管理动作要求可见页面和用户手势，默认拒绝且不接受外部窗口', async () => {
  let listener, reply, calls = 0;
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const activation = { isActive: false };
  const child = { postMessage: message => { reply = message; } };
  const frame = { contentWindow: child, checkVisibility: () => true };
  globalThis.window = { addEventListener: (_, fn) => { listener = fn; }, removeEventListener: () => {} };
  globalThis.document = { hasFocus: () => true, hidden: false };
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { userActivation: activation } });
  const event = { source: child, origin: 'null', data: { protocol: 'aio:plugin@2', kind: 'management-action', id: 'action', action: { kind: 'download-package', revision: 'a'.repeat(64) } } };
  try {
    let dispose = mountBridge(frame, () => assert.fail('动作进入执行器'));
    activation.isActive = true;
    await listener(event); assert(reply.error); dispose();
    dispose = mountBridge(frame, () => assert.fail('动作进入执行器'), { managementAction: async action => { calls++; assert.equal(action.kind, 'download-package'); return { status: 204, headers: [], body: [] }; } });
    await listener({ ...event, source: {} });
    await listener({ ...event, origin: 'https://foreign.test' });
    activation.isActive = false;
    await listener(event); assert(reply.error);
    activation.isActive = true;
    document.hidden = true;
    await listener(event); assert(reply.error);
    document.hidden = false;
    frame.checkVisibility = () => false;
    await listener(event); assert(reply.error);
    assert.equal(calls, 0);
    frame.checkVisibility = () => true;
    await listener(event); assert.equal(calls, 1); assert.equal(reply.response.status, 204);
    dispose(); await listener(event); assert.equal(calls, 1);
  } finally {
    delete globalThis.window; delete globalThis.document;
    if (previous) Object.defineProperty(globalThis, 'navigator', previous); else delete globalThis.navigator;
  }
});

test('客户端管理动作仅传递语义参数', async () => {
  let received, sent;
  const parent = { postMessage: message => { sent = message; } };
  const guest = { parent, addEventListener: (_, listener) => { received = listener; } };
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), guestContext(guest));
  for (const [operation, expected] of [
    [() => guest.aioPlugin.downloadPackage('a'.repeat(64)), { kind: 'download-package', revision: 'a'.repeat(64) }],
    [() => guest.aioPlugin.openSettings('component:source:settings'), { kind: 'open-settings', page_id: 'component:source:settings' }],
    [() => guest.aioPlugin.installTool('tool', '1.2.3'), { kind: 'install-tool', id: 'tool', version: '1.2.3' }],
  ]) {
    const waiting = operation();
    assert.equal(sent.kind, 'management-action');
    assert.deepEqual(JSON.parse(JSON.stringify(sent.action)), expected);
    received({ source: parent, data: { protocol: 'aio:plugin@2', kind: 'response', id: sent.id, response: { status: 204, headers: [], body: [] } } });
    assert.equal((await waiting).status, 204);
  }
});

test('文档图片通过独立消息读取并默认拒绝未开放宿主', async () => {
  let listener, reply, received, sent, calls = 0;
  globalThis.window = { addEventListener: (_, fn) => { listener = fn; }, removeEventListener: () => {} };
  const child = { postMessage: message => { reply = message; } };
  const parent = { postMessage: message => { sent = message; } };
  const guest = { parent, addEventListener: (_, fn) => { received = fn; } };
  vm.runInContext(readFileSync(new URL('./guest.js', import.meta.url), 'utf8'), guestContext(guest));
  const waiting = guest.aioPlugin.managementImage('https://images.example/a.png');
  const event = { source: child, origin: 'null', data: sent };
  try {
    let dispose = mountBridge({ contentWindow: child }, () => assert.fail('图片进入执行器'));
    await listener(event); assert(reply.error); dispose();
    dispose = mountBridge({ contentWindow: child }, () => assert.fail('图片进入执行器'), {
      managementImage: async url => { calls++; assert.equal(url, 'https://images.example/a.png'); return { status: 200, headers: [{ name: 'content-type', value: 'image/png' }], body: new Uint8Array([0, 255, 128]) }; },
    });
    await listener({ ...event, source: {} }); await listener({ ...event, origin: 'https://foreign.test' }); assert.equal(calls, 0);
    await listener(event); received({ source: parent, data: reply });
    assert.deepEqual([...(await waiting).body], [0, 255, 128]);
    dispose(); await listener(event); assert.equal(calls, 1);
    await assert.rejects(guest.aioPlugin.managementImage('x'.repeat(4097)), /Invalid image/);
  } finally { delete globalThis.window; }
});
