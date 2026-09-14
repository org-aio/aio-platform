(() => {
  const token = document.currentScript.dataset.token;
  let activated = new URL(location.href).searchParams.get('__aio_prepare') !== '1';
  let suspended = false;
  let prepared = false;
  let compiling = 0;
  let instantiated = 0;
  let settled = 0;
  const waiting = new Set();
  const notify = kind => parent.postMessage({ channel: 'aio-lifecycle', token, kind }, '*');
  const whenActive = () => {
    if (suspended) return Promise.reject(new Error('租户页面已暂停'));
    if (activated) return Promise.resolve();
    if (waiting.size >= 16) return Promise.reject(new Error('准备阶段请求超过限制'));
    return new Promise((resolve, reject) => waiting.add({ resolve, reject }));
  };
  addEventListener('message', event => {
    const message = event.data;
    if (event.source !== parent || message?.channel !== 'aio-lifecycle' || message.token !== token || message.kind !== 'state') return;
    suspended = message.suspended === true;
    activated ||= message.active === true && !suspended;
    if (activated || suspended) {
      for (const item of waiting) suspended ? item.reject(new Error('租户页面已暂停')) : item.resolve();
      waiting.clear();
    }
    dispatchEvent(new CustomEvent('aio:visibility', { detail: message.visible === true && !suspended }));
  });
  Object.defineProperty(window, 'aioLifecycle', { value: Object.freeze({ whenActive, get activated() { return activated; } }) });
  for (const name of ['instantiate', 'instantiateStreaming']) {
    const instantiate = globalThis.WebAssembly?.[name];
    if (typeof instantiate !== 'function') continue;
    WebAssembly[name] = async (...arguments_) => {
      compiling++;
      try {
        const result = await instantiate.apply(WebAssembly, arguments_);
        instantiated++;
        return result;
      } finally { compiling--; settled = Date.now(); }
    };
  }
  // 只观察初始化进度，不读取业务数据，也不调用插件服务。
  const inspect = () => {
    if (prepared || compiling || !document.body) return;
    const wasmReady = instantiated > 0 && Date.now() - settled >= 500 && document.body.childElementCount > 0;
    if (!wasmReady && !document.body.querySelector('canvas,button,input,[role="tree"]')) return;
    prepared = true;
    clearInterval(timer);
    notify('prepared');
  };
  const timer = setInterval(inspect, 250);
  addEventListener('pagehide', () => clearInterval(timer), { once: true });
  notify('ready');
})();
