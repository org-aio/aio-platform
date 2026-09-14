const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { spawn } = require('node:child_process');
const { createHash } = require('node:crypto');
const { mkdtemp, mkdir, writeFile, readFile, readdir, rm } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { resolve, join } = require('node:path');

const templates = resolve(__dirname, '../templates/toolchain');
const body = Buffer.from('verified tool archive\n');
const digest = createHash('sha256').update(body).digest('hex');
const unix = process.platform !== 'win32';

function shell(script, env, args = []) {
  return new Promise((resolve, reject) => {
    const child = spawn('sh', ['-eu', '-c', script, 'test', ...args], {
      env: { ...process.env, HTTP_PROXY: '', HTTPS_PROXY: '', ALL_PROXY: '',
        http_proxy: '', https_proxy: '', all_proxy: '', ...env },
    });
    let stdout = '', stderr = '';
    child.stdout.on('data', data => stdout += data);
    child.stderr.on('data', data => stderr += data);
    child.on('error', reject);
    child.on('close', code => resolve({ code, stdout: stdout.trim(), stderr }));
  });
}

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'aio network '));
  t.after(() => rm(root, { recursive: true, force: true }));
  const counts = new Map();
  const server = createServer((req, res) => {
    counts.set(req.url, (counts.get(req.url) || 0) + 1);
    if (req.url === '/range') {
      const offset = Number((req.headers.range || 'bytes=0-').match(/\d+/)[0]);
      if (offset === 0) {
        res.writeHead(200, { 'Content-Length': body.length, Connection: 'close' });
        res.end(body.subarray(0, 5));
      } else {
        res.writeHead(206, { 'Content-Length': body.length - offset, 'Content-Range': `bytes ${offset}-${body.length - 1}/${body.length}` });
        res.end(body.subarray(offset));
      }
    } else if (req.url === '/missing') { res.writeHead(404); res.end(); }
    else if (req.url === '/bad') { res.end('corrupted'); }
    else { setTimeout(() => res.end(body), 40); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => server.close(resolve)));
  const url = `http://127.0.0.1:${server.address().port}`;
  const env = { AIO_TOOLCHAIN_CACHE: root, AIO_NETWORK: 'china', AIO_OFFLINE: '0',
    AIO_DOWNLOAD_ROOT: '', SCRIPT: join(templates, 'download.sh') };
  const download = (urls, overrides = {}) => shell(
    '. "$SCRIPT"; aio_download tool.tar.gz "$1" "$2" "$3" "$4"',
    { ...env, ...overrides }, [digest, ...urls]);
  return { root, env, download, url, counts };
}

test('下载失败和损坏镜像会回退，只有校验成功的归档生效', { skip: !unix }, async t => {
  const f = await fixture(t);
  const result = await f.download([`${f.url}/good`, `${f.url}/missing`, `${f.url}/bad`]);
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(await readFile(result.stdout), body);
  assert.equal(f.counts.get('/missing'), 1);
  assert.equal(f.counts.get('/bad'), 1);
  assert.equal(f.counts.get('/good'), 1);
  assert.match(result.stderr, /摘要校验失败/);
  assert.deepEqual((await readdir(join(f.root, 'downloads'))).filter(name => /part|lock|owner/.test(name)), []);
});

test('离线模式复用完整缓存，拒绝损坏缓存且不联网', { skip: !unix }, async t => {
  const f = await fixture(t);
  const urls = [`${f.url}/good`, '', ''];
  const first = await f.download(urls);
  assert.equal(first.code, 0, first.stderr);
  assert.equal((await f.download(urls, { AIO_OFFLINE: '1' })).code, 0);
  await writeFile(first.stdout, 'partial');
  const broken = await f.download(urls, { AIO_OFFLINE: '1' });
  assert.notEqual(broken.code, 0);
  assert.match(broken.stderr, /离线缓存缺失或校验失败/);
  assert.equal(f.counts.get('/good'), 1);
});

test('两个进程并发下载只请求一次并共享完整归档', { skip: !unix }, async t => {
  const f = await fixture(t);
  const urls = [`${f.url}/good`, '', ''];
  const results = await Promise.all([f.download(urls), f.download(urls)]);
  for (const result of results) assert.equal(result.code, 0, result.stderr);
  assert.equal(results[0].stdout, results[1].stdout);
  assert.equal(f.counts.get('/good'), 1);
});

test('所有源校验失败不会发布缓存，global 不访问镜像', { skip: !unix }, async t => {
  const f = await fixture(t);
  const result = await f.download([`${f.url}/bad`, `${f.url}/good`, ''], { AIO_NETWORK: 'global' });
  assert.notEqual(result.code, 0);
  assert.equal(f.counts.get('/good'), undefined);
  assert.deepEqual(await readdir(join(f.root, 'downloads')), []);
});

test('企业镜像优先，死进程锁可自动恢复', { skip: !unix }, async t => {
  const f = await fixture(t);
  await mkdir(join(f.root, 'downloads'));
  await writeFile(join(f.root, 'downloads', `${digest}-tool.tar.gz.lock`), '99999999\n');
  const result = await f.download([`${f.url}/bad`, '', ''], { AIO_DOWNLOAD_ROOT: f.url });
  assert.equal(result.code, 0, result.stderr);
  assert.equal(f.counts.get('/tool.tar.gz'), 1);
  assert.equal(f.counts.get('/bad'), undefined);
});

test('连接中断后按 Range 续传并验证完整摘要', { skip: !unix }, async t => {
  const f = await fixture(t);
  const result = await f.download([`${f.url}/range`, '', '']);
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(await readFile(result.stdout), body);
  assert.equal(f.counts.get('/range'), 2);
});

test('JDK 路径支持空格，显式错误版本不会静默下载替换', { skip: !unix }, async t => {
  const root = await mkdtemp(join(tmpdir(), 'aio jdk '));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'bin'));
  for (const name of ['java', 'javac']) await writeFile(join(root, 'bin', name), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  const run = () => shell('. "$SCRIPT"; aio_find_jdk', { SCRIPT: join(templates, 'jdk.sh'), AIO_JAVA_HOME: root });
  await writeFile(join(root, 'release'), 'JAVA_VERSION="25.0.2"\n');
  const good = await run();
  assert.equal(good.code, 0, good.stderr);
  assert.equal(good.stdout, root);
  await writeFile(join(root, 'release'), 'JAVA_VERSION="24.0.1"\n');
  const bad = await run();
  assert.notEqual(bad.code, 0);
  assert.match(bad.stderr, /AIO_JAVA_HOME 必须指向完整的 JDK 25/);
});

test('工具锁文件没有重复记录且摘要完整', async () => {
  const lines = (await readFile(join(templates, 'artifacts.tsv'), 'utf8')).trim().split('\n').slice(1);
  const seen = new Set();
  for (const line of lines) {
    const [kind, platform, filename, hash, origin] = line.split('\t');
    assert.match(hash, /^[a-f0-9]{64}$/);
    assert.equal(new URL(origin).pathname.split('/').pop(), filename);
    assert.equal(seen.has(`${kind}/${platform}`), false);
    seen.add(`${kind}/${platform}`);
  }
});
