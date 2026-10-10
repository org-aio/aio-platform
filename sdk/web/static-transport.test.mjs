import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';

const source = await readFile(new URL('../../lib/plugin/host/src/runtime/server/static_transport.js', import.meta.url), 'utf8');
const publicOrigin = 'https://public.example';
const lanOrigin = 'https://lan.example:3443';
const asset = '/assets/app-dxh12345678.wasm';
function fixture(fetch) {
  const calls = [];
  const window = {fetch: async (...args) => { calls.push(args); return fetch(...args); }};
  runInNewContext(source, {window, document: {currentScript: {dataset: {lanOrigins: JSON.stringify([lanOrigin])}}},
    location: {origin: publicOrigin, href: `${publicOrigin}/?page=home`}, URL, AbortController, setTimeout, clearTimeout});
  return {fetch: window.fetch, calls};
}

test('首页指纹资产使用局域网且不携带账户 Cookie', async () => {
  const f = fixture(async () => new Response('wasm', {headers: {'content-type': 'application/wasm'}}));
  assert.equal(await (await f.fetch(asset)).text(), 'wasm');
  assert.equal(f.calls.length, 1);
  assert.equal(String(f.calls[0][0]), lanOrigin + asset);
  assert.equal(f.calls[0][1].credentials, 'omit');
  assert.equal(f.calls[0][1].redirect, 'error');
});

test('局域网错误、错误文档和中途断流均回退完整公网资源', async () => {
  for (const fail of [
    () => { throw new Error('unreachable'); },
    () => new Response('<html>', {headers: {'content-type': 'text/html'}}),
    () => new Response(new ReadableStream({start(controller) {controller.error(new Error('truncated'));}})),
  ]) {
    const f = fixture(async input => String(input).startsWith(lanOrigin) ? fail() : new Response('public'));
    assert.equal(await (await f.fetch(asset)).text(), 'public');
    assert.equal(f.calls.length, 2);
    assert.equal(f.calls[1][0], asset);
    await f.fetch(asset);
    assert.equal(f.calls.length, 3);
  }
});

test('账户 API、业务写入、外域和显式请求选项不参与静态选路', async () => {
  const f = fixture(async () => new Response('original'));
  for (const [input, options] of [
    ['/api/runtime/workers'], [asset, {method: 'POST'}], [asset, {headers: {authorization: 'fixture'}}],
    [asset, {credentials: 'include'}], [asset + '?token=fixture'], ['https://other.example' + asset],
    [new Request(publicOrigin + asset)], ['/assets/current.wasm'],
  ]) {
    await f.fetch(input, options);
    assert.equal(f.calls.at(-1)[0], input);
    assert.equal(f.calls.at(-1)[1], options);
  }
  assert.equal(f.calls.length, 8);
});

test('调用方取消后不发起公网重试', async () => {
  const controller = new AbortController();
  const f = fixture(async () => { controller.abort(); throw controller.signal.reason; });
  await assert.rejects(f.fetch(asset, {signal: controller.signal}), {name: 'AbortError'});
  assert.equal(f.calls.length, 1);
});
