(() => {
  const token = document.currentScript.dataset.token;
  const pending = new Map();
  const listeners = new Set();
  let sequence = 0;
  let restoredHash = null;
  let restoringScroll = null;
  let scrollTimer = null;
  let visible = window.aioLifecycle?.activated !== false;
  const call = payload => new Promise((resolve, reject) => {
    if (pending.size >= 16) return reject(new Error('导航请求超过限制'));
    const id = String(++sequence);
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error('导航请求超时')); }, 10000);
    pending.set(id, { resolve, reject, timeout });
    parent.postMessage({ channel: 'aio-navigation', token, id, ...payload }, '*');
  });
  const navigate = (fragment, { replace = false } = {}) => {
    if (typeof fragment !== 'string' || (fragment !== '' && !fragment.startsWith('#')) || fragment.length > 2048) {
      return Promise.reject(new TypeError('插件路由必须是有界片段'));
    }
    return call({ navigation: fragment, replace });
  };
  const onNavigationChange = listener => {
    if (typeof listener !== 'function') throw new TypeError('导航监听器必须是函数');
    listeners.add(listener);
    listener(location.hash);
    return () => listeners.delete(listener);
  };
  const notify = fragment => {
    for (const listener of listeners) {
      try { listener(fragment); } catch (error) { console.error(error); }
    }
  };
  const applyScroll = () => {
    if (!restoringScroll) return;
    if (Date.now() > restoringScroll.deadline) { restoringScroll = null; return; }
    let waiting = false;
    for (const [key, point] of restoringScroll.positions) {
      const element = key === 'window' ? document.scrollingElement : document.querySelector(`[data-url-scroll="${key}"]`);
      if (!element || element.scrollHeight < point.y + element.clientHeight || element.scrollWidth < point.x + element.clientWidth) { waiting = true; continue; }
      element.scrollLeft = point.x;
      element.scrollTop = point.y;
    }
    if (!waiting) {
      const completed = restoringScroll;
      requestAnimationFrame(() => { if (restoringScroll === completed) restoringScroll = null; });
    }
  };
  const restoreScroll = value => {
    const positions = new Map();
    for (const part of value.split(';').slice(0, 16)) {
      const match = /^guest-([a-zA-Z0-9_-]{1,34}):(\d{1,8}):(\d{1,8})$/.exec(part);
      if (match) positions.set(match[1], { x: Number(match[2]), y: Number(match[3]) });
    }
    if (!positions.has('window')) positions.set('window', { x: 0, y: 0 });
    for (const element of document.querySelectorAll('[data-url-scroll]')) {
      const key = element.dataset.urlScroll;
      if (/^[a-zA-Z0-9_-]{1,34}$/.test(key) && !positions.has(key)) positions.set(key, { x: 0, y: 0 });
    }
    restoringScroll = { positions, deadline: Date.now() + 15000 };
    requestAnimationFrame(applyScroll);
  };
  const observer = new MutationObserver(() => { if (restoringScroll) requestAnimationFrame(applyScroll); });
  observer.observe(document.documentElement, { childList: true, subtree: true });
  addEventListener('resize', applyScroll);
  addEventListener('aio:visibility', event => {
    visible = event.detail === true;
    if (!visible) { clearTimeout(scrollTimer); scrollTimer = null; }
  });
  for (const name of ['wheel', 'touchstart', 'pointerdown']) addEventListener(name, () => { restoringScroll = null; }, { passive: true });
  addEventListener('hashchange', () => {
    if (restoredHash !== null && location.hash === restoredHash) { restoredHash = null; return; }
    restoredHash = null;
    notify(location.hash);
    void navigate(location.hash).catch(error => console.error(error));
  });
  addEventListener('scroll', event => {
    if (!visible || (restoringScroll && Date.now() <= restoringScroll.deadline)) return;
    restoringScroll = null;
    const element = event.target === document ? document.scrollingElement : event.target;
    const key = element === document.scrollingElement ? 'window' : element?.dataset?.urlScroll;
    if (!key || !/^[a-zA-Z0-9_-]{1,34}$/.test(key)) return;
    clearTimeout(scrollTimer);
    scrollTimer = setTimeout(() => { scrollTimer = null; if (visible) void call({ scroll: { key, x: element.scrollLeft, y: element.scrollTop } }).catch(error => console.error(error)); }, 120);
  }, { capture: true, passive: true });
  addEventListener('message', event => {
    const message = event.data;
    if (event.source !== parent || message?.channel !== 'aio-navigation' || message.token !== token) return;
    if (typeof message.navigation === 'string' && (message.navigation === '' || message.navigation.startsWith('#')) && message.navigation.length <= 2048) {
      if (location.hash !== message.navigation) {
        restoredHash = message.navigation;
        location.replace(message.navigation || '#');
        notify(message.navigation);
      }
      if (typeof message.scroll === 'string' && message.scroll.length <= 2048) {
        clearTimeout(scrollTimer);
        scrollTimer = null;
        restoreScroll(message.scroll);
      }
      return;
    }
    const item = pending.get(message.id);
    if (!item) return;
    clearTimeout(item.timeout);
    pending.delete(message.id);
    if (message.error) item.reject(new Error(message.error)); else item.resolve(message.response);
  });
  Object.defineProperty(window, 'aioNavigation', { value: Object.freeze({ navigate, onNavigationChange }), writable: false, configurable: false });
})();
