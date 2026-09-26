const config = JSON.parse(await dioxus.recv());
const controller = new AbortController();
const leave = () => controller.abort();
window.addEventListener('pagehide', leave);
const prepare = async id => {
  dioxus.send(id);
  const deadline = Date.now() + 30000;
  while (!controller.signal.aborted && Date.now() < deadline) {
    const page = [...document.querySelectorAll('[data-aio-page]')].find(node => node.dataset.aioPage === id && node.dataset.aioWorkspaceContext === config.context);
    if (page?.querySelector('iframe[data-aio-prepared=true]')) return true;
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  return false;
};
const start = async () => {
  const deadline = Date.now() + 60000;
  while (!controller.signal.aborted && Date.now() < deadline) {
    const page = [...document.querySelectorAll('[data-aio-page-active="true"]')].find(node => node.dataset.aioWorkspaceContext === config.context);
    if (page?.querySelector('iframe[data-aio-prepared=true]')) {
      await warmFrontendAssets(config, controller.signal, prepare);
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
};
const timer = setTimeout(() => { void start().catch(() => {}); }, 0);
try { await dioxus.recv(); } finally {
  clearTimeout(timer); controller.abort();
  window.removeEventListener('pagehide', leave);
}
