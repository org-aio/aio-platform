import { test } from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFileSync } from 'node:fs';

// 直接取生产宿主运行时里真实拼接的函数，验证 base64 线协议，避免测试与实现漂移。
const source = readFileSync(new URL('../../lib/plugin/host/src/runtime/frontend_component.js', import.meta.url), 'utf8');
function extract(name, source) {
  // 定位 `const NAME = `，然后按大括号配对标到对应右大括号，取到完整箭头函数。
  const needle = `const ${name} = `;
  const start = source.indexOf(needle);
  assert.ok(start >= 0, `frontend_component.js 必须定义 ${name}`);
  let depth = 0;
  let i = source.indexOf('{', start);
  for (; i < source.length; i++) {
    const c = source[i];
    if (c === '{') depth++;
    else if (c === '}') { depth--; if (depth === 0) break; }
  }
  return source.slice(needle.length + start, i + 1);
}

const make = () => {
  const ctx = {
    btoa: (s) => Buffer.from(s, 'latin1').toString('base64'),
    atob: (s) => Buffer.from(s, 'base64').toString('latin1'),
  };
  const snippet = `const toBase64 = ${extract('toBase64', source)};\nconst fromBase64 = ${extract('fromBase64', source)};\nglobalThis.__out = { toBase64, fromBase64 };`;
  vm.runInNewContext(snippet, ctx);
  return ctx.__out;
};



test('toBase64 produces a base64 string and round-trips bytes', () => {
  const { toBase64, fromBase64 } = make();
  const bytes = new Uint8Array([0, 255, 128, 1, 2, 3]);
  const encoded = toBase64(bytes);
  assert.equal(typeof encoded, 'string', '传给 JSON 的必须是 base64 字符串，而不是字节数组');
  assert.deepEqual([...fromBase64(encoded)], [0, 255, 128, 1, 2, 3]);
});

test('toBase64 handles chunks above the 0x8000 slice boundary', () => {
  const { __r } = make();
  const { toBase64, fromBase64 } = make();
  const bytes = new Uint8Array(0x8000 + 10);
  for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251;
  assert.deepEqual([...fromBase64(toBase64(bytes))], [...bytes]);
});

test('fromBase64 rejects non-string and decodes large payloads', () => {
  const { fromBase64 } = make();
  assert.throws(() => fromBase64([0, 255]), /不匹配/);
  const big = new Uint8Array(1024 * 1024);
  for (let i = 0; i < big.length; i++) big[i] = (i * 7) % 256;
  const decoded = fromBase64(Buffer.from(big).toString('base64'));
  assert.equal(decoded.length, big.length);
  assert.deepEqual([...decoded], [...big]);
});
