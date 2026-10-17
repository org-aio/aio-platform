const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const url = new URL(config.src, location.href);
if (url.origin !== location.origin || !url.pathname.startsWith(`/api/runtime/components/assets/${config.token}/`)) throw new Error("插件资产地址无效");
const { mountBridge } = await import(`/api/runtime/components/bridge.js?v=${encodeURIComponent(config.host_version)}`);
const page = frame.closest('[data-aio-page-active]');
if (page?.dataset.aioPagePreparing === 'true') url.searchParams.set('__aio_prepare', '1');
let pageSuspended = false;
const active = () => !pageSuspended && (!page || page.dataset.aioWorkspaceActive !== 'false');
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
// 管理写入完成后先等待宿主提交新目录，旧页面不能抢在自身替换之前继续发请求。
const refreshCatalog = signal => new Promise((resolve, reject) => {
  const requestId = crypto.randomUUID();
  const finish = error => {
    clearTimeout(timeout);
    window.removeEventListener('aio:catalog-refreshed', refreshed);
    signal.removeEventListener('abort', aborted);
    if (error) reject(error); else resolve();
  };
  const refreshed = event => {
    if (!event.detail?.requests?.includes(requestId)) return;
    finish(event.detail.success ? null : new Error('操作已提交，但更新插件目录失败'));
  };
  const aborted = () => finish(new Error('插件页面已暂停或替换'));
  const timeout = setTimeout(() => finish(new Error('操作已提交，等待插件目录更新超时')), 15000);
  window.addEventListener('aio:catalog-refreshed', refreshed);
  signal.addEventListener('abort', aborted, { once: true });
  if (signal.aborted) return aborted();
  window.dispatchEvent(new CustomEvent('aio:catalog-invalidated', { detail: { requestId } }));
});
const invoke = async (request, operation = 'request') => {
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
    const response = await fetch(`/api/runtime/components/${requestTicket}/${operation}`, {
      method: 'POST', credentials: 'same-origin', redirect: 'error',
      headers: { 'content-type': 'application/json' }, body, signal: controller.signal,
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
    if (disposed || !active() || generation !== epoch || ticket !== requestTicket) throw new Error('租户页面已暂停');
    if (operation === 'manage' && request.method && !['GET', 'HEAD'].includes(request.method) && payload.data.status >= 200 && payload.data.status < 300) {
      await refreshCatalog(controller.signal);
      if (disposed || !active() || generation !== epoch || ticket !== requestTicket) throw new Error('插件页面已暂停或替换');
    }
    return payload.data;
  } finally { clearTimeout(timeout); pending.delete(controller); }
};
const disposeBridge = mountBridge(frame, invoke, {
  manage: request => invoke(request, 'manage'),
  downloadRequest: ({ path, name, query }) => {
    if (disposed || !lifecycle.active() || !active() || !visible() || !ticket || restoring) throw new Error('页面尚未就绪');
    const parameters = new URLSearchParams({ path, name });
    if (query != null) parameters.set('query', query);
    const link = document.createElement('a');
    link.href = `/api/runtime/components/${encodeURIComponent(ticket)}/download?${parameters}`;
    link.download = name;
    document.body.append(link); link.click(); link.remove();
    return { status: 204, headers: [], body: [] };
  },
  managementImage: async url => {
    if (!lifecycle.active() || !active()) throw new Error('页面已暂停');
    if (restoring) await restoring;
    const requestTicket = ticket;
    const generation = epoch;
    if (!requestTicket || disposed) throw new Error('页面已暂停');
    const controller = new AbortController();
    pending.add(controller);
    try {
      const response = await fetch(`/api/runtime/components/${requestTicket}/image`, {
        method: 'POST', credentials: 'same-origin', redirect: 'error',
        headers: { 'content-type': 'application/json' }, body: JSON.stringify({ url }),
        signal: AbortSignal.any([controller.signal, AbortSignal.timeout(18000)]),
      });
      if (!response.ok) throw new Error((await response.json()).error || `HTTP ${response.status}`);
      const body = new Uint8Array(await response.arrayBuffer());
      if (disposed || !active() || generation !== epoch || ticket !== requestTicket) throw new Error('页面已暂停');
      if (body.length > 10 * 1024 * 1024) throw new Error('图片超过大小限制');
      return { status: response.status, headers: [{ name: 'content-type', value: response.headers.get('content-type') }], body };
    } finally { pending.delete(controller); }
  },
  managementAction: async action => {
    if (disposed || !lifecycle.active() || !active() || !visible() || !ticket || restoring) throw new Error('页面尚未就绪');
    const requestTicket = ticket;
    const generation = epoch;
    const controller = new AbortController();
    pending.add(controller);
    try {
      const response = await fetch(`/api/runtime/components/${requestTicket}/action`, {
        method: 'POST', credentials: 'same-origin', redirect: 'error',
        headers: { 'content-type': 'application/json' }, body: JSON.stringify(action),
        signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
      });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
      if (disposed || !active() || !visible() || generation !== epoch || ticket !== requestTicket) throw new Error('页面已暂停');
      const approved = payload.data;
      if (approved.kind === 'open-settings') {
        window.dispatchEvent(new CustomEvent('aio:plugin-settings', { detail: { pageId: approved.page_id } }));
      } else if (approved.kind === 'download-package' && /^[a-f0-9]{64}$/.test(approved.revision)) {
        const link = document.createElement('a');
        link.href = `/api/runtime/components/${requestTicket}/packages/${approved.revision}`;
        link.download = `${approved.revision}.aio-plugin`;
        document.body.append(link); link.click(); link.remove();
      } else if (approved.kind === 'install-tool' && approved.url.startsWith('aio://install/')) {
        window.location.href = approved.url;
      } else throw new Error('宿主返回了未知操作');
      return { status: 204, headers: [], body: [] };
    } finally { pending.delete(controller); }
  },
  clipboard: true, fileDrop: () => lifecycle.active() && active() && visible() && ticket,
});
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
  window.removeEventListener('pageshow', resume);
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
const leave = event => {
  if (event.persisted) { pageSuspended = true; suspend(); } else cleanup();
};
const resume = () => { pageSuspended = false; activity(); };
document.addEventListener('visibilitychange', activity);
window.addEventListener('pagehide', leave);
window.addEventListener('pageshow', resume);
const heartbeat = setInterval(activity, 60000);
if ((page && page.dataset.aioWorkspaceContext !== config.context) || !active()) {
  cleanup(); dioxus.send({ error: '租户或权限已变化，请重新打开页面' });
} else frame.src = url.href;
try { await dioxus.recv(); } finally { cleanup(); }
