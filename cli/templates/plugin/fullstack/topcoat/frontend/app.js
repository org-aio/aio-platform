const output = document.querySelector('#counter');
const context = document.querySelector('#context');
const increment = document.querySelector('#increment');
const error = document.querySelector('#error');

async function api(method, path, body) {
  if (!window.aioPlugin) throw new Error('请从 AIO 工作空间打开插件');
  return window.aioPlugin.json(method, path, body);
}

async function load() {
  try {
    const [counter, identity] = await Promise.all([
      api('GET', '/api/counter'),
      api('GET', '/api/context')
    ]);
    output.textContent = String(counter.value);
    context.textContent = `租户 ${identity.tenant_id || '未提供'} · 用户 ${identity.user_id || '未提供'}`;
  } catch (cause) {
    error.textContent = String(cause.message || cause);
  }
}

increment.addEventListener('click', async () => {
  increment.disabled = true;
  error.textContent = '';
  try {
    const counter = await api('POST', '/api/counter', { increment: true });
    output.textContent = String(counter.value);
  } catch (cause) {
    error.textContent = String(cause.message || cause);
  } finally {
    increment.disabled = false;
  }
});

load();
