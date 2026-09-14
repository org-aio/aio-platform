(() => {
  const token = document.currentScript.dataset.token;
  const development = document.currentScript.dataset.development === 'true';
  const root = new URL('.', document.currentScript.src).href;
  const pending = new Map();
  let sequence = 0;
  let visible = true;
  const visibilityListeners = new Set();
  const call = payload => new Promise((resolve, reject) => {
    if (pending.size >= 16) return reject(new Error("请求数量超过限制"));
    const id = String(++sequence);
    const timeout = development && typeof payload.asset !== 'string' ? null : setTimeout(() => { pending.delete(id); reject(new Error("插件请求超时")); }, typeof payload.asset === 'string' ? 600000 : 30000);
    pending.set(id, { resolve, reject, timeout });
    parent.postMessage({ channel: "aio-plugin", token, id, ...payload }, "*");
  });
  const request = ({ method = 'GET', path, query = null, body = '' }) => {
    const send = () => call({ request: { method, path, query, body } });
    return window.aioLifecycle ? window.aioLifecycle.whenActive().then(send) : send();
  };
  addEventListener('hashchange', () => { void call({ navigation: location.hash }).catch(() => {}); });
  addEventListener("message", (event) => {
    if (event.source !== parent || event.data?.channel !== "aio-plugin" || event.data.token !== token) return;
    if (event.data.lifecycle === "visibility" && typeof event.data.visible === "boolean") {
      visible = event.data.visible;
      for (const listener of visibilityListeners) {
        try { listener(visible); } catch (error) { console.error(error); }
      }
      return;
    }
    const item = pending.get(event.data.id);
    if (!item) return;
    clearTimeout(item.timeout);
    pending.delete(event.data.id);
    if (event.data.error) item.reject(new Error(event.data.error));
    else item.resolve(event.data.asset || event.data.response);
  });
  const json = async (method, path, value) => {
    const response = await request({ method, path, body: value === undefined ? "" : JSON.stringify(value) });
    const result = response.body ? JSON.parse(response.body) : null;
    if (response.status < 200 || response.status >= 300) throw new Error(result?.error || `HTTP ${response.status}`);
    return result;
  };
  const onVisibilityChange = (listener) => {
    if (typeof listener !== "function") throw new TypeError("可见性监听器必须是函数");
    visibilityListeners.add(listener);
    listener(visible);
    return () => visibilityListeners.delete(listener);
  };
  Object.defineProperty(window, "aioPlugin", { value: Object.freeze({ request, json, get visible() { return visible; }, onVisibilityChange }), writable: false, configurable: false });
  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (input, options) => {
    const url = new URL(input instanceof Request ? input.url : input, document.baseURI);
    const method = options?.method || (input instanceof Request ? input.method : 'GET');
    if (!url.href.startsWith(root) || method !== 'GET') return nativeFetch(input, options);
    // 浏览器调试器需要原始流响应的资源地址，才能解析 Wasm 的相对源码映射。
    if (development && url.pathname.endsWith('.wasm')) return nativeFetch(input, options);
    if (options?.signal?.aborted) throw options.signal.reason;
    const path = decodeURIComponent(url.pathname.slice(new URL(root).pathname.length));
    const response = await call({ asset: path });
    if (options?.signal?.aborted) throw options.signal.reason;
    const result = new Response(response.bytes, { headers: { 'content-type': response.type } });
    // 模块加载器依赖响应地址解析 import.meta.url 和相对依赖。
    Object.defineProperty(result, 'url', { value: url.href });
    return result;
  };
  window.esmsInitOptions = { shimMode: true, nativePassthrough: false, fetch: window.fetch };
  // 保留动态生成 import map 的工具链语义，不修改插件源码。
  const maps = new MutationObserver(() => {
    for (const script of document.querySelectorAll('script[type="importmap"]')) script.type = 'importmap-shim';
  });
  maps.observe(document, { subtree: true, childList: true });
})();
