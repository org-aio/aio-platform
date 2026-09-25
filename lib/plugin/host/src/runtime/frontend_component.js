const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const url = new URL(config.src, location.href);
if (url.origin !== location.origin || !url.pathname.startsWith(`/api/runtime/components/assets/${config.token}/`)) throw new Error("插件资产地址无效");
const { mountBridge } = await import('/api/runtime/components/bridge.js');
const page = frame.closest('[data-aio-page-active]');
if (page?.dataset.aioPagePreparing === 'true') url.searchParams.set('__aio_prepare', '1');
const active = () => !page || page.dataset.aioWorkspaceActive !== 'false';
const visible = () => !document.hidden && (!page || page.dataset.aioPageActive === 'true');
let disposed = false;
let renewing = false;
let restoring = null;
let ticket = config.token;
let epoch = 0;
const pending = new Set();
const currentTicket = async () => {
  if (restoring) await restoring;
  return ticket;
};
const assets = mountFrontendAssets(frame, config, active, currentTicket);
const lifecycle = createFrontendLifecycle(frame, config);
const disposeBridge = mountBridge(frame, async request => {
  if (!lifecycle.active()) throw new Error('插件尚未激活');
  if (!active()) throw new Error('租户页面已暂停');
  if (restoring) await restoring;
  const requestTicket = ticket;
  if (!requestTicket) throw new Error('租户页面已暂停');
  const generation = epoch;
  const body = JSON.stringify({ ...request, body: Array.from(request.body ?? []) });
  if (!body || body.length > 32 * 1024 * 1024) throw new Error('请求超过大小限制');
  const controller = new AbortController();
  const timeout = config.development ? null : setTimeout(() => controller.abort(), 35000);
  pending.add(controller);
  try {
    const response = await fetch(`/api/runtime/components/${requestTicket}/request`, {
      method: 'POST', credentials: 'same-origin', redirect: 'error',
      headers: { 'content-type': 'application/json' }, body, signal: controller.signal,
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
    if (disposed || !active() || generation !== epoch || ticket !== requestTicket) throw new Error('租户页面已暂停');
    return payload.data;
  } finally { clearTimeout(timeout); pending.delete(controller); }
}, { clipboard: true });
const revoke = token => token && fetch(`/api/runtime/frontend/${token}`, { method: 'DELETE', credentials: 'same-origin', keepalive: true, redirect: 'error' }).catch(() => {});
const suspend = () => {
  epoch++;
  assets.abort();
  for (const request of pending) request.abort();
  const previous = ticket;
  ticket = null;
  void revoke(previous);
};
const restore = () => {
  if (ticket || restoring || disposed || !active()) return;
  const generation = epoch;
  const controller = new AbortController();
  restoring = (async () => {
    const response = await fetch('/api/runtime/frontend/mount', {
      method: 'POST', credentials: 'same-origin', redirect: 'error',
      headers: { 'content-type': 'application/json' }, body: JSON.stringify({ page_id: config.page_id }),
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(15000)]),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '恢复插件页面失败');
    const mount = result.data;
    if (disposed || !active() || generation !== epoch) { void revoke(mount.token); return; }
    if (mount.abi !== config.abi || mount.revision !== config.revision || mount.generation !== config.generation || mount.session_context !== config.session_context || mount.context !== config.context) {
      void revoke(mount.token);
      throw new Error('插件版本或登录上下文已变化');
    }
    ticket = mount.token;
  })().catch(error => {
    if (!disposed && active() && generation === epoch) {
      dioxus.send({ error: error.message });
      window.dispatchEvent(new Event('aio:catalog-invalidated'));
    }
  }).finally(() => {
    restoring = null;
    if (generation !== epoch && !ticket && active() && !disposed) restore();
  });
};
const cleanup = () => {
  if (disposed) return;
  disposed = true;
  clearInterval(heartbeat);
  disposeBridge();
  lifecycle.dispose();
  assets.dispose();
  observer.disconnect();
  document.removeEventListener('visibilitychange', activity);
  window.removeEventListener('pagehide', leave);
  window.removeEventListener('pageshow', activity);
  suspend();
  frame.removeAttribute('src');
};
const renew = async () => {
  if (disposed || renewing || !ticket || !active()) return;
  renewing = true;
  const requestTicket = ticket;
  const generation = epoch;
  const controller = new AbortController();
  pending.add(controller);
  try {
    const response = await fetch(`/api/runtime/components/${requestTicket}/renew`, {
      method: 'POST', credentials: 'same-origin', redirect: 'error',
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
    });
    if ([401,403,404].includes(response.status) && !disposed && active() && generation === epoch && ticket === requestTicket) {
      cleanup(); dioxus.send({ error: '插件挂载已失效，请重新打开页面' });
      window.dispatchEvent(new Event('aio:catalog-invalidated'));
    }
  } catch (_) {} finally { pending.delete(controller); renewing = false; }
};
const activity = () => {
  if (disposed) return;
  if (!active()) { if (ticket || restoring || pending.size) suspend(); return; }
  restore();
  if (visible()) void renew();
};
const observer = new MutationObserver(activity);
if (page) observer.observe(page, { attributes: true, attributeFilter: ['data-aio-page-active', 'data-aio-workspace-active'] });
const leave = event => { if (event.persisted) suspend(); else cleanup(); };
document.addEventListener('visibilitychange', activity);
window.addEventListener('pagehide', leave);
window.addEventListener('pageshow', activity);
const heartbeat = setInterval(activity, 60000);
if ((page && page.dataset.aioWorkspaceContext !== config.context) || !active()) {
  cleanup(); dioxus.send({ error: '租户或权限已变化，请重新打开页面' });
} else frame.src = url.href;
try { await dioxus.recv(); } finally { cleanup(); }
