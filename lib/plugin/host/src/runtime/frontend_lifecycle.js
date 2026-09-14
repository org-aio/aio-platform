function createFrontendLifecycle(frame, config) {
  const page = frame.closest('[data-aio-page-active]');
  let activated = false;
  let disposed = false;
  const available = () => !page || page.dataset.aioWorkspaceActive !== 'false';
  const visible = () => !document.hidden && available() && (!page || page.dataset.aioPageActive === 'true');
  const state = () => {
    if (disposed) return;
    if (!activated && visible()) {
      try {
        const recent = JSON.parse(sessionStorage.getItem('aio-plugin-recent') || '[]');
        sessionStorage.setItem('aio-plugin-recent', JSON.stringify([config.page_id, ...recent.filter(id => id !== config.page_id)].slice(0, 8)));
      } catch (_) {}
    }
    activated ||= visible();
    frame.contentWindow?.postMessage({ channel: 'aio-lifecycle', token: config.token, kind: 'state', active: activated && available(), visible: visible(), suspended: !available() }, '*');
  };
  const receive = event => {
    const message = event.data;
    if (disposed || event.source !== frame.contentWindow || event.origin !== 'null' || message?.channel !== 'aio-lifecycle' || message.token !== config.token) return;
    if (message.kind === 'ready') state();
    if (message.kind === 'prepared') frame.dataset.aioPrepared = 'true';
  };
  const observer = new MutationObserver(state);
  if (page) observer.observe(page, { attributes: true, attributeFilter: ['data-aio-page-active', 'data-aio-workspace-active'] });
  window.addEventListener('message', receive);
  document.addEventListener('visibilitychange', state);
  state();
  return {
    active: () => !disposed && activated && available(),
    dispose() { disposed = true; observer.disconnect(); window.removeEventListener('message', receive); document.removeEventListener('visibilitychange', state); },
  };
}
