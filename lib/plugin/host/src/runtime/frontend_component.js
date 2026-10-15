// 桥正文按 base64 走 JSON：字节数组会放大到约 3.4 倍，浏览器还得解析一座数字山。
const toBase64 = bytes => {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return btoa(binary);
};
const fromBase64 = value => {
  if (typeof value !== 'string') throw new Error('宿主与插件运行时不匹配，请刷新页面后重试');
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
};
const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const url = new URL(config.src, location.href);
if (url.origin !== location.origin || !url.pathname.startsWith(`/api/runtime/components/assets/${config.token}/`)) throw new Error("插件资产地址无效");
const { mountBridge } = await import(`/api/runtime/components/bridge.js?v=${encodeURIComponent(config.host_version)}`);
const page = frame.closest('[data-aio-page-active]');
if (page?.dataset.aioPagePreparing === 'true') url.searchParams.set('__aio_prepare', '1');
const active = () => !page || page.dataset.aioWorkspaceActive !== 'false';
const visible = () => !document.hidden && (!page || page.dataset.aioPageActive === 'true');
const navigation = mountFrontendNavigation(frame, config, active, visible);
if (navigation.fragment) url.hash = navigation.fragment;
let disposed = false;
let renewing = false;
let restoring = null;
let releasing = Promise.resolve();
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
  const body = JSON.stringify({ ...request, body: toBase64(request.body ?? new Uint8Array()) });
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
    return { ...payload.data, body: fromBase64(payload.data.body) };
  } finally { clearTimeout(timeout); pending.delete(controller); }
}, { clipboard: true, fileDrop: () => lifecycle.active() && active() && visible() && ticket });
const revoke = token => {
  if (!token) return Promise.resolve();
  const request = fetch(`/api/runtime/frontend/${token}`, { method: 'DELETE', credentials: 'same-origin', keepalive: true, redirect: 'error' }).then(() => {}, () => {});
  releasing = request;
  return request;
};
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
    await releasing;
    if (disposed || !active() || generation !== epoch) return;
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
    }
  }).finally(() => {
    restoring = null;
    if (generation !== epoch && !ticket && active() && !disposed) restore();
  });
};
const recoverLease = () => {
  if (disposed || restoring || !active()) return;
  suspend();
  restore();
};
const cleanup = () => {
  if (disposed) return;
  disposed = true;
  clearInterval(heartbeat);
  disposeBridge();
  lifecycle.dispose();
  navigation.dispose();
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
      // 票据过期只替换挂载凭证，不能销毁已加载的 iframe 和插件内存状态。
      recoverLease();
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
