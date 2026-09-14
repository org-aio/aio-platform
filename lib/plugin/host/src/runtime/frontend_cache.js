function createFrontendAssetCache(config) {
  const prefix = 'aio-plugin-assets-v1-';
  const name = prefix + config.session_context;
  const limit = 96 * 1024 * 1024;
  const shared = window[Symbol.for('aio.asset-cache')] ||= { downloads: new Map(), writes: Promise.resolve() };
  const inflight = shared.downloads;
  const storage = typeof caches === 'undefined' ? Promise.resolve(null) : caches.open(name).catch(() => null);
  const digest = async bytes => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), byte => byte.toString(16).padStart(2, '0')).join('');
  if (typeof caches !== 'undefined') {
    void caches.keys().then(names => Promise.all(names.filter(key => key.startsWith(prefix) && key !== name).map(key => caches.delete(key)))).catch(() => {});
  }
  const save = (cache, key, bytes, type) => {
    shared.writes = shared.writes.catch(() => {}).then(async () => {
      let size = bytes.byteLength;
      const keys = await cache.keys();
      const sizes = await Promise.all(keys.map(async key => Number((await cache.match(key))?.headers.get('content-length') || 0)));
      size += sizes.reduce((sum, value) => sum + value, 0);
      while (keys.length && (size > limit || keys.length >= 512)) {
        await cache.delete(keys.shift());
        size -= sizes.shift();
      }
      await cache.put(key, new Response(bytes, { headers: { 'content-type': type, 'content-length': String(bytes.byteLength) } }));
    });
    return shared.writes;
  };
  const download = async (url, signal) => {
    for (let attempt = 0; ; attempt++) {
      signal?.throwIfAborted();
      try {
        const timeout = AbortSignal.timeout(120000);
        const response = await fetch(url, { credentials: 'same-origin', redirect: 'error', priority: config.background ? 'low' : 'high', signal: signal ? AbortSignal.any([signal, timeout]) : timeout });
        if (response.status === 429 || response.status >= 500) throw new Error(`读取插件资源失败: HTTP ${response.status}`);
        if (!response.ok) return { error: `读取插件资源失败: HTTP ${response.status}` };
        return { bytes: await response.arrayBuffer(), type: response.headers.get('content-type') || 'application/octet-stream' };
      } catch (error) {
        if (signal?.aborted || attempt === 2) throw error;
        await new Promise(resolve => setTimeout(resolve, 1000 * (attempt + 1)));
      }
    }
  };
  const load = async (path, ticket, signal) => {
    signal?.throwIfAborted();
    if (!Object.hasOwn(config.assets, path)) throw new Error('插件未声明该资源');
    const expected = config.assets[path];
    const extension = path.match(/\.[^./]+$/)?.[0].toLowerCase() || '';
    const key = new URL(`/_aio_cache/${expected}/asset${encodeURIComponent(extension)}`, location.origin).href;
    const pendingKey = name + key;
    const pending = inflight.get(pendingKey);
    if (pending) {
      try {
        const result = await pending.operation;
        signal?.throwIfAborted();
        return result;
      } catch (error) {
        signal?.throwIfAborted();
        // 后台任务取消后，前台使用自己的有效票据接管下载。
        if (!pending.signal?.aborted) throw error;
        if (inflight.get(pendingKey) === pending) inflight.delete(pendingKey);
        return load(path, ticket, signal);
      }
    }
    const operation = (async () => {
      const cache = await storage;
      const cached = await cache?.match(key).catch(() => null);
      if (cached) {
        const bytes = await cached.arrayBuffer();
        if (await digest(bytes) === expected) {
          signal?.throwIfAborted();
          return { bytes, type: cached.headers.get('content-type') };
        }
        await cache.delete(key).catch(() => {});
      }
      signal?.throwIfAborted();
      const route = config.abi === 2 ? 'components' : 'frontend';
      const result = await download(`/api/runtime/${route}/assets/${ticket}/${path.split('/').map(encodeURIComponent).join('/')}`, signal);
      if (result.error) throw new Error(result.error);
      const { bytes, type } = result;
      if (bytes.byteLength > limit || await digest(bytes) !== expected) throw new Error('插件资源摘要或大小校验失败');
      signal?.throwIfAborted();
      if (cache) await save(cache, key, bytes, type).catch(() => {});
      return { bytes, type };
    })();
    const entry = { operation, signal };
    inflight.set(pendingKey, entry);
    try { return await operation; }
    finally { if (inflight.get(pendingKey) === entry) inflight.delete(pendingKey); }
  };
  return load;
}
