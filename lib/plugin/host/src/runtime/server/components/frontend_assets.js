(() => {
  const { token, root, development } = document.currentScript.dataset;
  const pending = new Map();
  let sequence = 0;
  addEventListener('message', event => {
    const message = event.data;
    if (event.source !== parent || message?.channel !== 'aio-assets' || message.token !== token) return;
    const item = pending.get(message.id);
    if (!item) return;
    pending.delete(message.id);
    clearTimeout(item.timeout);
    if (message.error) item.reject(new Error(message.error));
    else item.resolve(message);
  });
  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (input, options) => {
    const url = new URL(input instanceof Request ? input.url : input, document.baseURI);
    const method = options?.method || (input instanceof Request ? input.method : 'GET');
    if (!url.href.startsWith(root) || method !== 'GET') return nativeFetch(input, options);
    if (development === 'true' && url.pathname.endsWith('.wasm')) return nativeFetch(input, options);
    const signal = options?.signal || (input instanceof Request ? input.signal : undefined);
    signal?.throwIfAborted();
    const assetPath = decodeURIComponent(url.pathname.slice(new URL(root).pathname.length));
    const result = await new Promise((resolve, reject) => {
      if (pending.size >= 16) return reject(new Error('插件资源请求超过限制'));
      const id = String(++sequence);
      // 资源缺失或链路中断时立即结束等待并回报宿主，页面不再停在无限加载态。
      const timeout = setTimeout(() => {
        pending.delete(id);
        parent.postMessage({ channel: 'aio-lifecycle', token, kind: 'error', message: `插件资源加载超时：${assetPath}` }, '*');
        reject(new Error('插件资源请求超时'));
      }, 30000);
      pending.set(id, { resolve, reject, timeout });
      parent.postMessage({ channel: 'aio-assets', token, id, path: assetPath }, '*');
    });
    signal?.throwIfAborted();
    const response = new Response(result.bytes, { headers: { 'content-type': result.type } });
    Object.defineProperty(response, 'url', { value: url.href });
    return response;
  };
  window.esmsInitOptions = { shimMode: true, nativePassthrough: false, fetch: window.fetch };
  // Compose 工具链会在经典脚本中动态插入 import map。
  new MutationObserver(() => {
    for (const script of document.querySelectorAll('script[type="importmap"]')) script.type = 'importmap-shim';
  }).observe(document, { subtree: true, childList: true });
})();
