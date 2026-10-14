import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {runInNewContext} from 'node:vm';

const source = await readFile(new URL('../../lib/plugin/host/src/runtime/server/static_transport.js', import.meta.url), 'utf8');
const publicOrigin = 'https://public.example';
const lanOrigin = 'https://lan.example:3443';
const asset = '/assets/app-dxh12345678.wasm';
const stylesheet = '/assets/style-dxh87654321.css';
const probePath = '/api/runtime/transport';

function fixture(fetch, origins = [lanOrigin]) {
  const calls = [];
  const window = {fetch: async (...args) => { calls.push(args); return fetch(...args); }};
  runInNewContext(source, {window, document: {currentScript: {dataset: {lanOrigins: JSON.stringify(origins)}}},
    location: {origin: publicOrigin, href: `${publicOrigin}/?page=home`}, URL, AbortController, setTimeout, clearTimeout});
  return {fetch: window.fetch, calls};
}

// 允许探测完成（并发的 Promise 链），不引入真实等待。
const settled = () => new Promise(resolve => setTimeout(resolve, 0));

test('局域网不可达时首屏指纹资产仍直接返回公网副本', async () => {
  const f = fixture(async input => {
    if (String(input).startsWith(lanOrigin)) throw new Error('unreachable');
    return new Response('public', {headers: {'content-type': 'application/wasm'}});
  });
  const response = await f.fetch(asset);
  assert.equal(await response.text(), 'public');
  // 关键路径只有一次公网请求；探测在旁路并发。
  assert.equal(f.calls.filter(([input]) => input === asset).length, 1);
  assert.equal(String(f.calls.at(-1)[0]), asset);
});

test('局域网可用后后续指纹资产改取局域网且不携带账户 Cookie', async () => {
  const f = fixture(async input => {
    if (String(input).startsWith(lanOrigin) && new URL(input).pathname === probePath) return new Response('{}');
    if (String(input).startsWith(lanOrigin)) return new Response('lan', {headers: {'content-type': 'application/wasm'}});
    return new Response('public');
  });
  await f.fetch(asset);
  await settled();
  const routed = await f.fetch(stylesheet);
  assert.equal(await routed.text(), 'lan');
  assert.equal(String(f.calls.at(-1)[0]), lanOrigin + stylesheet);
  assert.equal(f.calls.at(-1)[1].credentials, 'omit');
  assert.equal(f.calls.at(-1)[1].redirect, 'error');
});

test('局域网错误、错误文档和中途断流均回退完整公网资源', async () => {
  for (const fail of [
    () => { throw new Error('unreachable'); },
    () => new Response('<html>', {headers: {'content-type': 'text/html'}}),
    () => new Response(new ReadableStream({start(controller) {controller.error(new Error('truncated'));}})),
  ]) {
    const f = fixture(async input => String(input).startsWith(lanOrigin) && new URL(input).pathname === probePath
      ? new Response('{}') : String(input).startsWith(lanOrigin) ? fail() : new Response('public'));
    await f.fetch(asset);
    await settled();
    assert.equal(await (await f.fetch(stylesheet)).text(), 'public');
    assert.equal(f.calls.at(-1)[0], stylesheet);
  }
});

test('一次探测结论在全页共享，失败后进入冷却不再重复探测', async () => {
  let probes = 0;
  const f = fixture(async input => {
    if (String(input).startsWith(lanOrigin)) { probes += 1; throw new Error('unreachable'); }
    return new Response('public');
  });
  await f.fetch(asset);
  await settled();
  await f.fetch(stylesheet);
  await f.fetch('/assets/other-dxhabcdef01.js');
  await settled();
  assert.equal(probes, 1);
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

test('未配置局域网来源时完全不改写请求', async () => {
  const f = fixture(async () => new Response('original'), []);
  await f.fetch(asset);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0][0], asset);
  assert.equal(f.calls[0][1], undefined);
});

test('调用方取消局域网副本后不发起公网重试', async () => {
  const controller = new AbortController();
  let routed = false;
  const f = fixture(async input => {
    if (String(input).startsWith(lanOrigin) && new URL(input).pathname === probePath) return new Response('{}');
    if (String(input).startsWith(lanOrigin)) {
      routed = true;
      controller.abort();
      throw controller.signal.reason;
    }
    return new Response('public');
  });
  await f.fetch(asset);
  await settled();
  const before = f.calls.length;
  await assert.rejects(f.fetch(stylesheet, {signal: controller.signal}), {name: 'AbortError'});
  assert.equal(routed, true);
  assert.equal(f.calls.length, before + 1);
});
