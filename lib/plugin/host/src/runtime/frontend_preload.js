async function warmFrontendAssets(config, signal, prepare = async () => {}) {
  if (typeof caches === 'undefined' || navigator.connection?.saveData || ['slow-2g', '2g'].includes(navigator.connection?.effectiveType)) return;
  const seen = new Set();
  const resources = new Set();
  let budget = 80 * 1024 * 1024;
  let count = 0;
  let prepared = 0;
  const instanceLimit = typeof matchMedia === 'function' && matchMedia('(max-width: 767px)').matches ? 1 : 2;
  let recent = [];
  try { recent = JSON.parse(sessionStorage.getItem('aio-plugin-recent') || '[]'); } catch (_) {}
  const pages = [...config.pages].sort((a, b) => {
    const rank = page => recent.includes(page.id) ? recent.indexOf(page.id) : recent.length;
    return rank(a) - rank(b);
  });
  const pause = () => new Promise(resolve => setTimeout(resolve, 100));
  for (const page of pages) {
    signal.throwIfAborted();
    while (document.visibilityState === 'hidden') { signal.throwIfAborted(); await pause(); }
    let mount;
    try {
      const response = await fetch('/api/runtime/frontend/mount', {
        method: 'POST', credentials: 'same-origin', redirect: 'error', priority: 'low',
        headers: { 'content-type': 'application/json' }, body: JSON.stringify({ page_id: page.id }),
        signal: AbortSignal.timeout(30000),
      });
      if ([401, 403].includes(response.status)) return;
      if (!response.ok) continue;
      mount = (await response.json()).data;
      signal.throwIfAborted();
      const version = mount.abi === 2 ? `${mount.revision}:${mount.generation}` : JSON.stringify([JSON.parse(page.version)[0], mount.revision, mount.generation]);
      if (mount.session_context !== config.session_context || mount.context !== config.context || version !== page.version) return;
      if (seen.has(mount.revision)) continue;
      seen.add(mount.revision);
      const load = createFrontendAssetCache({ ...mount, background: true });
      const paths = Object.keys(mount.assets).filter(path => !/\.(html?|map)$/i.test(path) && (/\.(mjs|wasm|otf|ttf|woff2?)$/i.test(path) || mount.asset_sizes[path] >= 256 * 1024));
      // 预热模块和大资源，避免包内未使用的源码碎片占满队列。
      paths.sort((a, b) => mount.asset_sizes[a] - mount.asset_sizes[b]);
      let complete = true;
      const queue = paths.filter(path => {
        const extension = path.match(/\.[^./]+$/)?.[0].toLowerCase() || '';
        const key = `${mount.assets[path]}/${extension}`;
        if (resources.has(key)) return false;
        const size = mount.asset_sizes[path];
        if (!Number.isSafeInteger(size) || size < 0 || size > budget || count >= 480) { complete = false; return false; }
        budget -= size; count++;
        resources.add(key);
        return true;
      });
      const worker = async () => {
        while (queue.length) {
          signal.throwIfAborted();
          while (document.visibilityState === 'hidden') { signal.throwIfAborted(); await pause(); }
          await load(queue.shift(), mount.token, signal);
          await pause();
        }
      };
      const results = await Promise.allSettled([worker(), worker()]);
      if (complete && results.every(result => result.status === 'fulfilled') && prepared < instanceLimit) {
        prepared++;
        if (await prepare(page.id) === false) prepared = instanceLimit;
      }
    } catch (_) {
      signal.throwIfAborted();
    } finally {
      if (mount?.token) await fetch(`/api/runtime/frontend/${mount.token}`, { method: 'DELETE', credentials: 'same-origin', keepalive: true, signal: AbortSignal.timeout(10000) }).catch(() => {});
    }
  }
}
