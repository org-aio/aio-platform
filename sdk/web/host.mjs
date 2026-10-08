export function mountBridge(frame, invoke, options = {}) {
  const disposeFileDrop = options.fileDrop ? mountFileDrop(frame, options.fileDrop) : () => {};
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
  return () => { active = false; disposeFileDrop(); window.removeEventListener("message", listener); };
}

// 文件系统目录只能由真实来源的宿主读取，不能放宽插件的不透明沙箱。
function mountFileDrop(frame, permit) {
  let subscription;
  let armed = false;
  let timeout;
  let epoch = 0;
  let disposed = false;
  const allowed = () => !disposed && frame.isConnected && frame.checkVisibility() && !document.hidden && permit();
  const reset = () => {
    clearTimeout(timeout);
    if (armed) { frame.inert = false; }
    armed = false;
  };
  const contains = event => {
    const rect = frame.getBoundingClientRect();
    return event.clientX >= rect.left && event.clientX < rect.right && event.clientY >= rect.top && event.clientY < rect.bottom;
  };
  const refresh = () => { clearTimeout(timeout); timeout = setTimeout(reset, 1500); };
  const arm = () => {
    armed = true;
    frame.inert = true;
    refresh();
  };
  const message = event => {
    const data = event.data;
    if (event.source !== frame.contentWindow || event.origin !== "null" || data?.protocol !== "aio:plugin@2" || typeof data.id !== "string" || data.id.length > 80) { return; }
    if (data.kind === "file-drop-subscribe") {
      reset(); epoch++;
      subscription = data.enabled === true ? data.id : undefined;
    }
    if (data.kind === "file-drag" && data.id === subscription && allowed() && (!frame.inert || armed)) {
      // 暂时将命中目标交给宿主；iframe 保持挂载，界面和会话不会重建。
      arm();
    }
  };
  const enter = event => {
    if (event.isTrusted && subscription && allowed() && contains(event) && (!frame.inert || armed) && event.dataTransfer?.types.includes("Files")) { arm(); }
  };
  const over = event => {
    if (!armed) { enter(event); }
    if (!armed) { return; }
    if (!allowed() || !contains(event)) { reset(); return; }
    event.preventDefault();
    refresh();
  };
  const leave = event => { if (armed && !contains(event)) { reset(); } };
  const drop = async event => {
    if (!armed) { return; }
    const lease = allowed();
    const source = frame.contentWindow;
    const id = subscription;
    const generation = ++epoch;
    const rect = frame.getBoundingClientRect();
    const valid = () => generation === epoch && id === subscription && source === frame.contentWindow && lease && allowed() === lease;
    reset();
    if (!event.isTrusted || !lease || !contains(event) || !event.dataTransfer?.files.length) { return; }
    event.preventDefault();
    event.stopImmediatePropagation();
    // 原事件结束后拖拽数据失效，先保存 File/Entry 引用及有界的文本。
    const items = Array.from(event.dataTransfer.items).filter(item => item.kind === "file").map(item => ({ file: item.getAsFile(), entry: item.webkitGetAsEntry?.() }));
    const text = {};
    for (const type of ["text/plain", "text/html", "text/uri-list"]) {
      const value = event.dataTransfer.getData(type);
      if (value && value.length <= 100000 && !/file:\/\//i.test(value)) { text[type] = value; }
    }
    const point = { x: (event.clientX - rect.left) * frame.clientWidth / rect.width, y: (event.clientY - rect.top) * frame.clientHeight / rect.height };
    const modifiers = { ctrlKey: event.ctrlKey, altKey: event.altKey, shiftKey: event.shiftKey, metaKey: event.metaKey };
    let count = 0;
    let bytes = 0;
    const read = operation => new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("读取拖入目录超时")), 30000);
      const finish = callback => value => { clearTimeout(timer); callback(value); };
      try { operation(finish(resolve), finish(reject)); }
      catch (error) { clearTimeout(timer); reject(error); }
    });
    const account = (file) => {
      if (!valid()) { throw new Error("文件拖入已随页面或设备连接关闭"); }
      if (++count > 4096) { throw new Error("目录内的文件或子目录数量超过限制"); }
      if (file) {
        bytes += file.size;
        if (file.size > 128 * 1024 * 1024 || bytes > 512 * 1024 * 1024) { throw new Error("文件拖入大小超过限制"); }
      }
    };
    async function walk(directory, prefix, entries, depth) {
      if (depth > 32) { throw new Error("目录层级超过限制"); }
      const reader = directory.createReader();
      while (true) {
        if (!valid()) { throw new Error("文件拖入已随页面或设备连接关闭"); }
        const batch = await read((resolve, reject) => reader.readEntries(resolve, reject));
        if (!batch.length) { return; }
        for (const entry of batch) {
          const relativePath = prefix + entry.name;
          if (relativePath.split("/").length > 32 || new TextEncoder().encode(relativePath).length > 4096) { throw new Error("目录内的相对路径超过限制"); }
          if (entry.isDirectory) {
            account(); entries.push({ kind: "directory", relativePath });
            await walk(entry, relativePath + "/", entries, depth + 1);
          } else {
            const file = await read((resolve, reject) => entry.file(resolve, reject));
            account(file); entries.push({ kind: "file", relativePath, file });
          }
        }
      }
    }
    const payload = { protocol: "aio:plugin@2", kind: "file-drop", id, point, modifiers, text };
    try {
      const roots = [];
      for (const {file, entry} of items) {
        if (entry?.isDirectory) {
          account();
          const entries = [];
          await walk(entry, "", entries, 0);
          roots.push({ kind: "directory", name: entry.name, entries });
        } else if (file) { account(file); roots.push({ kind: "file", file }); }
      }
      if (valid()) { source.postMessage({ ...payload, roots }, "*"); }
    } catch (error) {
      if (valid()) { source.postMessage({ ...payload, error: String(error.message ?? error) }, "*"); }
    }
  };
  const load = () => { reset(); epoch++; subscription = undefined; };
  window.addEventListener("message", message);
  window.addEventListener("dragenter", enter, true);
  window.addEventListener("dragover", over, true);
  window.addEventListener("dragleave", leave, true);
  window.addEventListener("drop", drop, true);
  window.addEventListener("dragend", reset, true);
  frame.addEventListener("load", load);
  return () => {
    disposed = true; load();
    window.removeEventListener("message", message);
    window.removeEventListener("dragenter", enter, true);
    window.removeEventListener("dragover", over, true);
    window.removeEventListener("dragleave", leave, true);
    window.removeEventListener("drop", drop, true);
    window.removeEventListener("dragend", reset, true);
    frame.removeEventListener("load", load);
  };
}
