const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const assetURL = new URL(config.src, window.location.href);
if (assetURL.origin !== window.location.origin || !assetURL.pathname.startsWith(`/api/runtime/frontend/assets/${config.token}/`)) {
  throw new Error("插件资产地址无效");
}
const requests = new Map();
const asset = createFrontendAssetCache(config);
let disposed = false;
let renewing = false;
let ticket = config.token;
let epoch = 0;
let restoring = null;
let releasing = Promise.resolve();
const page = frame.closest("[data-aio-page-active]");
if (page?.dataset.aioPagePreparing === 'true') assetURL.searchParams.set('__aio_prepare', '1');
const lifecycle = createFrontendLifecycle(frame, config);
const workspaceActive = () => !page || page.dataset.aioWorkspaceActive !== 'false';
const visible = () => !document.hidden && (!page || page.dataset.aioPageActive === "true");
const routeKey = `aio-plugin-route:${config.session_context}:${config.context}:${config.page_id}`;
const routes = window.__aioPluginRoutes || (window.__aioPluginRoutes = new Map());
const reply = (message, transfer = []) => {
  if (!disposed) frame.contentWindow?.postMessage({ channel: "aio-plugin", token: config.token, ...message }, "*", transfer);
};
const receive = async (event) => {
  const message = event.data;
  if (disposed || event.source !== frame.contentWindow || event.origin !== "null" || !message ||
      message.channel !== "aio-plugin" || message.token !== config.token ||
      typeof message.id !== "string" || !/^[0-9]{1,16}$/.test(message.id)) return;
  if (requests.has(message.id)) return;
  if (!workspaceActive()) return reply({ id: message.id, error: '租户页面已暂停' });
  if (typeof message.navigation === 'string' && (message.navigation === '' || message.navigation.startsWith('#')) && message.navigation.length <= 2048) {
    routes.set(routeKey, message.navigation);
    return reply({ id: message.id, response: message.navigation });
  }
  if (requests.size >= 16) return reply({ id: message.id, error: "插件请求超过并发配额" });
  let body;
  try {
    if (typeof message.asset !== 'string') {
      if (!lifecycle.active()) throw new Error('插件尚未激活');
      body = JSON.stringify(message.request);
      if (!body || body.length > 1048576) throw new Error("插件请求体过大");
    }
  } catch (error) {
    return reply({ id: message.id, error: error.message });
  }
  const controller = new AbortController();
  const timeout = config.development && typeof message.asset !== 'string' ? null : setTimeout(() => controller.abort(), typeof message.asset === 'string' ? 540000 : 25000);
  requests.set(message.id, controller);
  const generation = epoch;
  try {
    if (restoring) await restoring;
    if (!ticket || disposed || !workspaceActive() || generation !== epoch) throw new Error('租户页面已暂停');
    if (typeof message.asset === 'string') {
      const output = await asset(message.asset, ticket, controller.signal);
      if (generation === epoch && !controller.signal.aborted && workspaceActive()) {
        const bytes = output.bytes.slice(0);
        reply({ id: message.id, asset: { bytes, type: output.type } }, [bytes]);
      }
      return;
    }
    const response = await fetch(`/api/runtime/frontend/${ticket}/request`, {
      method: "POST", credentials: "same-origin", redirect: "error",
      headers: { "content-type": "application/json" }, body, signal: controller.signal,
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
    if (generation === epoch && workspaceActive()) reply({ id: message.id, response: payload.data });
  } catch (error) {
    reply({ id: message.id, error: error.message });
  } finally {
    clearTimeout(timeout);
    requests.delete(message.id);
  }
};
window.addEventListener("message", receive);
const visibility = () => {
  reply({ lifecycle: "visibility", visible: visible() });
};
const revoke = token => {
  if (!token) return Promise.resolve();
  const request = fetch(`/api/runtime/frontend/${token}`, {
    method: "DELETE", credentials: "same-origin", keepalive: true, redirect: "error",
  }).then(() => {}, () => {});
  releasing = request;
  return request;
};
const suspend = () => {
  epoch++;
  for (const [id, controller] of requests) {
    controller.abort();
    reply({ id, error: '租户页面已暂停' });
  }
  requests.clear();
  const previous = ticket;
  ticket = null;
  void revoke(previous);
};
const restore = () => {
  if (ticket || restoring || disposed || !workspaceActive()) return;
  const generation = epoch;
  restoring = (async () => {
    await releasing;
    if (disposed || !workspaceActive() || generation !== epoch) return;
    const response = await fetch('/api/runtime/frontend/mount', {
      method: 'POST', credentials: 'same-origin', redirect: 'error',
      headers: { 'content-type': 'application/json' }, body: JSON.stringify({ page_id: config.page_id }),
      signal: AbortSignal.timeout(15000),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '恢复租户页面失败');
    const mount = result.data;
    if (disposed || !workspaceActive() || generation !== epoch) { void revoke(mount.token); return; }
    if (mount.revision !== config.revision || mount.generation !== config.generation || mount.session_context !== config.session_context || mount.context !== config.context) {
      void revoke(mount.token);
      throw new Error('插件版本或登录上下文已变化');
    }
    ticket = mount.token;
    visibility();
  })().catch(error => {
    if (!disposed && workspaceActive() && generation === epoch) {
      dioxus.send({ error: error.message });
    }
  }).finally(() => {
    restoring = null;
    if (generation !== epoch && !ticket && workspaceActive() && !disposed) restore();
  });
};
const cleanup = () => {
  if (disposed) return;
  disposed = true;
  clearInterval(heartbeat);
  observer.disconnect();
  lifecycle.dispose();
  document.removeEventListener("visibilitychange", activity);
  window.removeEventListener("pagehide", leave);
  window.removeEventListener("pageshow", activity);
  window.removeEventListener("message", receive);
  frame.removeEventListener("load", visibility);
  suspend();
  frame.removeAttribute("src");
};
const renew = async () => {
  if (disposed || renewing || !ticket || !workspaceActive()) return;
  renewing = true;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10000);
  const generation = epoch;
  requests.set("lease", controller);
  try {
    const response = await fetch(`/api/runtime/frontend/${ticket}/renew`, {
      method: "POST", credentials: "same-origin", redirect: "error", signal: controller.signal,
    });
    if ([401, 403, 404].includes(response.status) && !disposed && generation === epoch && workspaceActive()) {
      cleanup();
      dioxus.send({ error: "插件挂载已失效，请重新打开页面" });
      window.dispatchEvent(new Event("aio:catalog-invalidated"));
    }
  } catch (_) {
    // 临时断网不销毁已加载界面；服务端仍逐次校验业务请求权限。
  } finally {
    clearTimeout(timeout);
    requests.delete("lease");
    renewing = false;
  }
};
const activity = () => {
  visibility();
  if (!workspaceActive()) { if (ticket || requests.size) suspend(); return; }
  restore();
  if (visible()) void renew();
};
const leave = event => { if (!event.persisted) cleanup(); };
const observer = new MutationObserver(activity);
if (page) observer.observe(page, { attributes: true, attributeFilter: ["data-aio-page-active", "data-aio-workspace-active"] });
document.addEventListener("visibilitychange", activity);
window.addEventListener("pagehide", leave);
window.addEventListener("pageshow", activity);
frame.addEventListener("load", visibility);
const heartbeat = setInterval(renew, 60000);
if ((page && page.dataset.aioWorkspaceContext !== config.context) || !workspaceActive()) {
  cleanup();
  dioxus.send({ error: '挂载期间租户或权限已变化，请重新打开页面' });
} else {
  const fragment = routes.get(routeKey);
  if (fragment) assetURL.hash = fragment;
  frame.src = assetURL.href;
}
try {
  await dioxus.recv();
} finally {
  cleanup();
}
