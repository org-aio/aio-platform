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
