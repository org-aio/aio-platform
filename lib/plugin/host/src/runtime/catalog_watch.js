// 持续监听并保留尚未提交的请求，重载期间的并发写入不能被旧目录确认。
const pending = new Set();
const changed = event => {
  const requestId = event.detail?.requestId;
  if (typeof requestId === 'string') pending.add(requestId);
  dioxus.send([...pending]);
};
const refreshed = event => {
  for (const requestId of event.detail?.requests ?? []) pending.delete(requestId);
};
const poll = () => dioxus.send(null);
const timer = setInterval(poll, 30000);
window.addEventListener('focus', poll);
window.addEventListener('aio:catalog-invalidated', changed);
window.addEventListener('aio:catalog-refreshed', refreshed);
try { await dioxus.recv(); } finally {
  clearInterval(timer);
  window.removeEventListener('focus', poll);
  window.removeEventListener('aio:catalog-invalidated', changed);
  window.removeEventListener('aio:catalog-refreshed', refreshed);
}
