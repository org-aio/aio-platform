export function mountBridge(frame, invoke, options = {}) {
  let active = true;
  let inflight = 0;
  const listener = async (event) => {
    const message = event.data;
    if (!active || event.source !== frame.contentWindow || event.origin !== "null" || message?.protocol !== "aio:plugin@2" || !["request", "clipboard", "download"].includes(message.kind)) return;
    if (typeof message.id !== "string" || message.id.length > 80) return;
    const source = event.source;
    const reply = { protocol: "aio:plugin@2", kind: "response", id: message.id };
    try {
      if (inflight >= 16) throw new Error("Too many pending requests");
      inflight++;
      try {
        if (message.kind === "clipboard") {
          if (!options.clipboard || !document.hasFocus() || !navigator.userActivation?.isActive || typeof message.text !== "string" || message.text.length > 100000) throw new Error("Clipboard access denied");
          await navigator.clipboard.writeText(message.text);
          reply.response = { status: 204, headers: [], body: [] };
        } else if (message.kind === "download") {
          // 只接受可见插件中的当前用户手势，不允许后台页面触发下载。
          if (options.download === false || !document.hasFocus() || !navigator.userActivation?.isActive || !frame.checkVisibility?.() ||
              typeof message.name !== "string" || !message.name.length || message.name.length > 255 || /[\\/\x00-\x1f]/.test(message.name) ||
              !(message.body instanceof Uint8Array) || message.body.length > 16 * 1024 * 1024 ||
              typeof message.mime !== "string" || message.mime.length > 200 || /[\r\n]/.test(message.mime)) {
            throw new Error("Download access denied");
          }
          const url = URL.createObjectURL(new Blob([message.body], { type: message.mime }));
          const link = document.createElement("a");
          link.href = url;
          link.download = message.name;
          document.body.append(link);
          link.click();
          link.remove();
          setTimeout(() => URL.revokeObjectURL(url), 60000);
          reply.response = { status: 204, headers: [], body: [] };
        } else reply.response = await invoke(message.request);
      }
      finally { inflight--; }
    } catch (cause) { reply.error = String(cause.message ?? cause); }
    if (active && source === frame.contentWindow) source.postMessage(reply, "*");
  };
  window.addEventListener("message", listener);
  return () => { active = false; window.removeEventListener("message", listener); };
}
