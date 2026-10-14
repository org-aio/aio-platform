(() => {
  const origins = JSON.parse(document.currentScript.dataset.lanOrigins);
  const originalFetch = window.fetch.bind(window);
  // 探测与公网请求并发进行，任何失败都只影响是否改写后续请求，绝不计入首屏关键路径。
  const PROBE_TIMEOUT_MS = 1500;
  const COOLDOWN_MS = 300_000;
  const LAN = origins.filter(origin => origin !== location.origin);

  // 结论在一次页面生命周期内共享：undefined=未决，null=不可用，字符串=可用来源。
  let decision;
  let pending;
  let retryAfter = 0;

  const eligible = url =>
    url.origin === location.origin && !url.search && !url.username && !url.password &&
    /^\/assets\/[^/]+-dxh[0-9a-fA-F]{8,}\.[^/]+$/.test(url.pathname);

  const probe = () => {
    if (pending) return pending;
    pending = (async () => {
      for (const origin of LAN) {
        const deadline = new AbortController();
        const timeout = setTimeout(() => deadline.abort(), PROBE_TIMEOUT_MS);
        try {
          const response = await originalFetch(new URL('/api/runtime/transport', origin), {
            method: 'GET', credentials: 'omit', mode: 'cors', cache: 'no-store',
            signal: deadline.signal,
          });
          await response.body?.cancel();
          if (response.ok) return origin;
        } catch {
          // 局域网不可达只影响后续选路；公网副本已经在并发返回。
        } finally {
          clearTimeout(timeout);
        }
      }
      retryAfter = Date.now() + COOLDOWN_MS;
      return null;
    })().then(value => {
      decision = value;
      pending = undefined;
      return value;
    });
    return pending;
  };

  // 仅为无凭据的指纹静态资产选路，账户 API、写操作和显式请求选项保持原语义。
  window.fetch = async (input, options) => {
    if (!(typeof input === 'string' || input instanceof URL) ||
        (options && Object.keys(options).some(key => !['method', 'signal', 'cache'].includes(key))) ||
        (options?.method && options.method !== 'GET')) {
      return originalFetch(input, options);
    }
    const url = new URL(input, location.href);
    if (!LAN.length || !eligible(url) || Date.now() < retryAfter) {
      return originalFetch(input, options);
    }
    // 已确认可用的局域网来源：按原语义改取局域网副本，失败后回退同一个公网 GET。
    const route = typeof decision === 'string' ? decision : undefined;
    if (route) {
      try {
        const response = await originalFetch(new URL(url.pathname, route), {
          method: 'GET', credentials: 'omit', mode: 'cors', redirect: 'error',
          cache: options?.cache ?? 'default', signal: options?.signal,
        });
        if (!response.ok || response.headers.get('content-type')?.startsWith('text/html')) {
          await response.body?.cancel();
          throw new Error('局域网副本不可用');
        }
        // 完整接收后再交给调用方，局域网中途断流仍可安全回退同一个静态 GET。
        await response.clone().arrayBuffer();
        options?.signal?.throwIfAborted();
        return response;
      } catch {
        options?.signal?.throwIfAborted();
        decision = undefined;
        retryAfter = Date.now() + COOLDOWN_MS;
        return originalFetch(input, options);
      }
    }
    // 未决或不可用时同样先取公网副本，结论由并发的探测补齐给后续资产。
    if (decision === undefined && Date.now() >= retryAfter) probe();
    return originalFetch(input, options);
  };
})();
