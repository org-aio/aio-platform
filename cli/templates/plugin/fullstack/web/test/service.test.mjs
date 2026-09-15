import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, copyFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createServer } from 'node:net';
import { test } from 'node:test';

test('独立 artifact 提供健康检查、页面定义与真实业务 API', { timeout: 60000 }, async () => {
  const socket = createServer();
  await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
  const port = socket.address().port;
  await new Promise(resolve => socket.close(resolve));
  const directory = await mkdtemp(join(tmpdir(), 'aio-test-'));
  await copyFile('dist/server.cjs', join(directory, 'server.cjs'));
  const child = spawn(process.execPath, ['server.cjs'], {
    cwd: directory, env: { ...process.env, AIO_PLUGIN_PORT: String(port) }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  const exit = once(child, 'exit');
  let log = '';
  child.stdout.on('data', data => log += data);
  child.stderr.on('data', data => log += data);
  const base = `http://127.0.0.1:${port}`;
  try {
    let ready = false;
    for (let attempt = 0; attempt < 200; attempt++) {
      if (child.exitCode !== null) throw new Error(log);
      try { ready = (await fetch(`${base}/health`)).ok; } catch {}
      if (ready) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.ok(ready, log);
    assert.equal((await (await fetch(`${base}/aio/definition`)).json())[0].body.entry, 'index.html');
    const post = value => fetch(`${base}/api/counter`, {
      method: 'POST', headers: { 'content-type': 'application/json', 'x-aio-tenant-id': 'test-tenant' },
      body: JSON.stringify(value),
    });
    const response = await post({ value: 41 });
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { value: 42, tenant_id: 'test-tenant' });
    for (const value of [{ value: '41' }, {}, null, { value: Number.MAX_SAFE_INTEGER }]) {
      assert.equal((await post(value)).status, 400);
    }
    const malformed = await fetch(`${base}/api/counter`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: '{',
    });
    assert.equal(malformed.status, 400);
  } finally {
    child.kill('SIGTERM');
    const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
    await exit;
    clearTimeout(timer);
    await rm(directory, { recursive: true, force: true });
  }
});
