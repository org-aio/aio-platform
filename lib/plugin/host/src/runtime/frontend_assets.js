function mountFrontendAssets(frame, config, active) {
  const load = createFrontendAssetCache(config);
  const pending = new Map();
  let disposed = false;
  const receive = async event => {
    const message = event.data;
    if (event.source !== frame.contentWindow || event.origin !== 'null' || message?.channel !== 'aio-assets' || message.token !== config.token) return;
    if (typeof message.id !== 'string' || message.id.length > 80 || pending.has(message.id)) return;
    const reply = payload => {
      if (!disposed) frame.contentWindow?.postMessage({ channel: 'aio-assets', token: config.token, id: message.id, ...payload }, '*', payload.bytes ? [payload.bytes] : []);
    };
    if (!active() || pending.size >= 16) return reply({ error: '插件资源请求已暂停或超过限制' });
    const controller = new AbortController();
    pending.set(message.id, controller);
    try {
      const result = await load(message.path, config.token, controller.signal);
      if (!active() || controller.signal.aborted) throw new Error('插件资源请求已暂停');
      reply({ bytes: result.bytes.slice(0), type: result.type });
    } catch (error) { reply({ error: error.message }); }
    finally { pending.delete(message.id); }
  };
  window.addEventListener('message', receive);
  const abort = () => { for (const controller of pending.values()) controller.abort(); };
  return {
    abort,
    dispose() { disposed = true; abort(); window.removeEventListener('message', receive); },
  };
}
