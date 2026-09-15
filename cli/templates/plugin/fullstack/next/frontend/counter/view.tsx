"use client";
import { useState } from 'react';
import { increment } from '../../shared/counter/service';
import { incrementOnServer } from '../../shared/counter/client';

export function Counter() {
  const [local, setLocal] = useState(0);
  const [remote, setRemote] = useState(0);
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  async function request() {
    setBusy(true);
    try {
      const result = await incrementOnServer(remote);
      setRemote(result.value);
      setMessage(`租户：${result.tenant_id}`);
    } catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }
  return <main>
    <h1>{"__TITLE__"}</h1>
    <section><h2>前端计数</h2><output aria-label="前端计数">{local}</output><button onClick={() => setLocal(increment({ value: local }))}>前端 +1</button></section>
    <section><h2>后端计数</h2><output aria-label="后端计数">{remote}</output><button disabled={busy} onClick={request}>后端 +1</button><p role="status">{message}</p></section>
  </main>;
}
