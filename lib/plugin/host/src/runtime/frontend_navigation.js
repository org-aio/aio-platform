function mountFrontendNavigation(frame, config, active, visible) {
  let disposed = false;
  const reply = message => {
    if (!disposed) frame.contentWindow?.postMessage({ channel: 'aio-navigation', token: config.token, ...message }, '*');
  };
  const restore = () => {
    if (disposed || !active() || !visible() || !window.__adminUrlState) return;
    reply({ navigation: window.__adminUrlState.parameter(config.page_id, 'route') || '', scroll: window.__adminUrlState.scroll(config.page_id) });
  };
  const receive = event => {
    const message = event.data;
    if (disposed || event.source !== frame.contentWindow || event.origin !== 'null' || message?.channel !== 'aio-navigation' || message.token !== config.token || typeof message.id !== 'string' || !/^[0-9]{1,16}$/.test(message.id)) return;
    if (!active() || !visible()) return reply({ id: message.id, error: '插件页面未激活' });
    if (typeof message.navigation === 'string' && (message.navigation === '' || message.navigation.startsWith('#')) && message.navigation.length <= 2048) {
      window.__adminUrlState?.update({ page: config.page_id, values: { route: message.navigation }, mode: message.replace === true ? 'replace' : 'push' });
      if (typeof message.history_scroll === 'string' && message.history_scroll.length <= 2048) {
        for (const part of message.history_scroll.split(';').slice(0, 16)) {
          const position = /^guest-([a-zA-Z0-9_-]{1,34}):(\d{1,8}):(\d{1,8})$/.exec(part);
          if (position) window.__adminUrlState?.recordScroll({ page: config.page_id, key: `guest-${position[1]}`, x: Number(position[2]), y: Number(position[3]) });
        }
      }
      restore();
      return reply({ id: message.id, response: message.navigation });
    }
    if (message.scroll && typeof message.scroll.key === 'string' && /^[a-zA-Z0-9_-]{1,34}$/.test(message.scroll.key) && Number.isFinite(message.scroll.x) && Number.isFinite(message.scroll.y)) {
      window.__adminUrlState?.recordScroll({ page: config.page_id, key: `guest-${message.scroll.key}`, x: message.scroll.x, y: message.scroll.y });
      return reply({ id: message.id, response: null });
    }
    reply({ id: message.id, error: '插件导航参数无效' });
  };
  window.addEventListener('message', receive);
  window.addEventListener('admin-url-state', restore);
  frame.addEventListener('load', restore);
  const page = frame.closest('[data-aio-page-active]');
  const observer = new MutationObserver(restore);
  if (page) observer.observe(page, { attributes: true, attributeFilter: ['data-aio-page-active', 'data-aio-workspace-active'] });
  return { fragment: window.__adminUrlState?.parameter(config.page_id, 'route') || '', dispose() {
    disposed = true;
    observer.disconnect();
    window.removeEventListener('message', receive);
    window.removeEventListener('admin-url-state', restore);
    frame.removeEventListener('load', restore);
  } };
}
