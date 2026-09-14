const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const url = new URL(config.src, location.href);
if (url.origin !== location.origin || !url.pathname.startsWith(`/api/runtime/components/assets/${config.token}/`)) throw new Error("插件资产地址无效");
const { mountBridge } = await import('/api/runtime/components/bridge.js');
const page = frame.closest('[data-aio-page-active]');
if (page?.dataset.aioPagePreparing === 'true') url.searchParams.set('__aio_prepare', '1');
const active = () => !page || page.dataset.aioWorkspaceActive !== 'false';
let disposed = false;
let renewing = false;
const pending = new Set();
const assets = mountFrontendAssets(frame, config, active);
const lifecycle = createFrontendLifecycle(frame, config);
const disposeBridge = mountBridge(frame, async request => {
  if (!lifecycle.active()) throw new Error('插件尚未激活');
  if (!active()) throw new Error('租户页面已暂停');
  const body = JSON.stringify({ ...request, body: Array.from(request.body ?? []) });
  if (!body || body.length > 32 * 1024 * 1024) throw new Error('请求超过大小限制');
  const controller = new AbortController();
  const timeout = config.development ? null : setTimeout(() => controller.abort(), 35000);
  pending.add(controller);
  try {
    const response = await fetch(`/api/runtime/components/${config.token}/request`, {
      method: 'POST', credentials: 'same-origin', redirect: 'error',
      headers: { 'content-type': 'application/json' }, body, signal: controller.signal,
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
    if (disposed || !active()) throw new Error('租户页面已暂停');
    return payload.data;
  } finally { clearTimeout(timeout); pending.delete(controller); }
}, { clipboard: true });
const cleanup = () => {
  if (disposed) return;
  disposed = true;
  clearInterval(heartbeat);
  disposeBridge();
  lifecycle.dispose();
  assets.dispose();
  observer.disconnect();
  window.removeEventListener('pagehide', leave);
  for (const request of pending) request.abort();
  frame.removeAttribute('src');
  void fetch(`/api/runtime/frontend/${config.token}`, { method: 'DELETE', credentials: 'same-origin', keepalive: true }).catch(() => {});
};
const observer = new MutationObserver(() => {
  if (!active()) {
    assets.abort();
    for (const request of pending) request.abort();
  }
});
if (page) observer.observe(page, { attributes: true, attributeFilter: ['data-aio-workspace-active'] });
const leave = event => { if (!event.persisted) cleanup(); };
window.addEventListener('pagehide', leave);
const heartbeat = setInterval(async () => {
  if (disposed || renewing || !active()) return;
  renewing = true;
  try {
    const response = await fetch(`/api/runtime/components/${config.token}/renew`, { method: 'POST', credentials: 'same-origin', redirect: 'error', signal: AbortSignal.timeout(10000) });
    if ([401,403,404].includes(response.status)) {
      cleanup(); dioxus.send({ error: '插件挂载已失效，请重新打开页面' });
      window.dispatchEvent(new Event('aio:catalog-invalidated'));
    }
  } catch (_) {} finally { renewing = false; }
}, 60000);
if ((page && page.dataset.aioWorkspaceContext !== config.context) || !active()) {
  cleanup(); dioxus.send({ error: '租户或权限已变化，请重新打开页面' });
} else frame.src = url.href;
try { await dioxus.recv(); } finally { cleanup(); }
