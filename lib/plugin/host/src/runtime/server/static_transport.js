(() => {
  const origins = JSON.parse(document.currentScript.dataset.lanOrigins);
  const originalFetch = window.fetch.bind(window);
  let preferred;
  let retryAfter = 0;

  // 仅为无凭据的指纹静态资产选路，账户 API、写操作和显式请求选项保持原语义。
  window.fetch = async (input, options) => {
    if (!(typeof input === 'string' || input instanceof URL) ||
        (options && Object.keys(options).some(key => !['method', 'signal', 'cache'].includes(key))) ||
        (options?.method && options.method !== 'GET')) {
      return originalFetch(input, options);
    }
    const url = new URL(input, location.href);
    if (url.origin !== location.origin || url.search || url.username || url.password ||
        !/^\/assets\/[^/]+-dxh[0-9a-fA-F]{8,}\.[^/]+$/.test(url.pathname) || Date.now() < retryAfter) {
      return originalFetch(input, options);
    }
    const candidates = preferred ? [preferred] : origins.filter(origin => origin !== location.origin);
    for (const origin of candidates) {
      options?.signal?.throwIfAborted();
      const deadline = new AbortController();
      const timeout = setTimeout(() => deadline.abort(), 2000);
      const cancel = () => deadline.abort(options.signal.reason);
      options?.signal?.addEventListener('abort', cancel, {once: true});
      try {
        const response = await originalFetch(new URL(url.pathname, origin), {
          method: 'GET', credentials: 'omit', mode: 'cors', redirect: 'error',
          cache: options?.cache ?? 'default', signal: deadline.signal,
        });
        if (!response.ok || response.headers.get('content-type')?.startsWith('text/html')) {
          await response.body?.cancel();
          continue;
        }
        // 完整接收后再交给调用方，局域网中途断流仍可安全回退同一个静态 GET。
        await response.clone().arrayBuffer();
        options?.signal?.throwIfAborted();
        preferred = origin;
        return response;
      } catch {
        options?.signal?.throwIfAborted();
      } finally {
        clearTimeout(timeout);
        options?.signal?.removeEventListener('abort', cancel);
      }
    }
    preferred = undefined;
    retryAfter = Date.now() + 30_000;
    return originalFetch(input, options);
  };
})();
