(() => {
  "use strict";
  const deviceRoot = document.currentScript?.dataset.root;
  const lanOrigins = JSON.parse(document.currentScript?.dataset.lanOrigins || "[]");
  let selectedRoot;
  let checked = 0;
  let selecting;
  const blockedOrigins = new Map();
  const deviceTransport = async () => {
    if (!deviceRoot) throw new Error("宿主未提供设备视图通道");
    if (selectedRoot && Date.now() - checked < 30000) return selectedRoot;
    if (selecting) return selecting;
    selecting = (async () => {
      const canonical = new URL(deviceRoot);
      selectedRoot = canonical;
      // 候选由宿主注入；只读探测不发送 Cookie 或设备票据。
      for (const candidate of lanOrigins) {
        if ((blockedOrigins.get(candidate) || 0) > Date.now()) continue;
        try {
          const origin = new URL(candidate);
          if (origin.protocol !== "https:" || origin.origin !== candidate) continue;
          const probe = await fetch(new URL("/api/runtime/transport", origin), { credentials: "omit", redirect: "error", cache: "no-store", signal: AbortSignal.timeout(1800) });
          const offer = (await probe.json()).data;
          if (!probe.ok || offer.public_origin !== canonical.origin || !offer.lan_origins.includes(candidate)) continue;
          selectedRoot = new URL(canonical.pathname, origin);
          break;
        } catch (_) {}
      }
      checked = Date.now();
      return selectedRoot;
    })().finally(() => { selecting = null; });
    return selecting;
  };
  const deviceView = async (request) => {
    const root = await deviceTransport();
    const response = await window.fetch(new URL("__device_view", root), { method: "POST", headers: { "content-type": "text/plain" }, body: JSON.stringify(request), credentials: "omit", redirect: "error", signal: AbortSignal.timeout(35000) }).catch(error => {
      // 不重放已提交的操作；下次连接重新探测入口。
      if (root.origin !== new URL(deviceRoot).origin) blockedOrigins.set(root.origin, Date.now() + 30000);
      selectedRoot = null; checked = 0; throw error;
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || `HTTP ${response.status}`);
    return result.data;
  };
  const development = document.currentScript?.dataset.development === "true";
  const pending = new Map();
  let fileDrop;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const createId = () => {
    if (typeof globalThis.crypto?.randomUUID === "function") return globalThis.crypto.randomUUID();
    const bytes = new Uint8Array(16);
    if (typeof globalThis.crypto?.getRandomValues === "function") globalThis.crypto.getRandomValues(bytes);
    else for (let index = 0; index < bytes.length; index++) bytes[index] = Math.floor(Math.random() * 256);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  };
  window.addEventListener("message", (event) => {
    const message = event.data;
    if (fileDrop && event.source === window.parent && message?.protocol === "aio:plugin@2" && message.kind === "file-drop" && message.id === fileDrop.id) {
      fileDrop.listener(message);
      return;
    }
    if (event.source !== window.parent || message?.protocol !== "aio:plugin@2" || message.kind !== "response") return;
    const call = pending.get(message.id);
    if (!call) return;
    pending.delete(message.id);
    clearTimeout(call.timer);
    if (message.error) call.reject(new Error(message.error));
    else call.resolve({ ...message.response, body: new Uint8Array(message.response.body) });
  });
  const sendRequest = (input) => new Promise((resolve, reject) => {
    if (pending.size >= 16) return reject(new Error("Too many pending requests"));
    if (!input || typeof input.path !== "string" || !input.path.startsWith("/") || input.path.startsWith("//")) return reject(new Error("Invalid service path"));
    const url = new URL(input.path, "https://aio.invalid");
    if (url.origin !== "https://aio.invalid" || url.hash) return reject(new Error("Invalid service path"));
    if (url.search && input.query != null) return reject(new Error("Specify query only once"));
    const body = input.body ?? new Uint8Array();
    if (!(body instanceof Uint8Array) || body.length > 16 * 1024 * 1024) return reject(new Error("Invalid binary body"));
    const id = createId();
    const timer = development ? null : setTimeout(() => { pending.delete(id); reject(new Error("Service request timed out")); }, 35000);
    pending.set(id, { resolve, reject, timer });
    window.parent.postMessage({ protocol: "aio:plugin@2", kind: "request", id,
      request: { method: input.method ?? "GET", path: url.pathname, query: input.query ?? (url.search.slice(1) || null),
        headers: input.headers ?? [], body } }, "*");
  });
  const request = input => window.aioLifecycle
    ? window.aioLifecycle.whenActive().then(() => sendRequest(input))
    : sendRequest(input);
  const json = async (method, path, value) => {
    const response = await request({ method, path,
      headers: value === undefined ? [] : [{ name: "content-type", value: "application/json" }],
      body: value === undefined ? new Uint8Array() : encoder.encode(JSON.stringify(value)) });
    if (response.status < 200 || response.status >= 300) throw new Error(decoder.decode(response.body));
    return response.body.length ? JSON.parse(decoder.decode(response.body)) : null;
  };
  const copy = (text) => new Promise((resolve, reject) => {
    if (pending.size >= 16 || typeof text !== "string" || text.length > 100000) return reject(new Error("Invalid clipboard request"));
    const id = createId();
    const timer = setTimeout(() => { pending.delete(id); reject(new Error("Clipboard request timed out")); }, 5000);
    pending.set(id, { resolve, reject, timer });
    window.parent.postMessage({ protocol: "aio:plugin@2", kind: "clipboard", id, text }, "*");
  });
  // 下载由宿主执行，沙箱始终保持禁止直接下载和顶层导航。
  const download = (name, body, mime = "application/octet-stream") => new Promise((resolve, reject) => {
    if (pending.size >= 16 || typeof name !== "string" || !name.length || name.length > 255 || /[\\/\x00-\x1f]/.test(name) ||
        !(body instanceof Uint8Array) || body.length > 16 * 1024 * 1024 || typeof mime !== "string" || mime.length > 200 || /[\r\n]/.test(mime)) {
      return reject(new Error("Invalid download request"));
    }
    const id = createId();
    const timer = setTimeout(() => { pending.delete(id); reject(new Error("Download request timed out")); }, 5000);
    pending.set(id, { resolve, reject, timer });
    window.parent.postMessage({ protocol: "aio:plugin@2", kind: "download", id, name, mime, body }, "*");
  });
  const navigate = (fragment, options) => window.aioNavigation.navigate(fragment, options);
  const onNavigationChange = listener => window.aioNavigation.onNavigationChange(listener);
  const onFileDrop = listener => {
    if (typeof listener !== "function") { throw new TypeError("文件拖入监听器必须是函数"); }
    const registration = { id: createId(), listener };
    fileDrop = registration;
    window.parent.postMessage({ protocol: "aio:plugin@2", kind: "file-drop-subscribe", id: registration.id, enabled: true }, "*");
    return () => {
      if (fileDrop !== registration) { return; }
      fileDrop = undefined;
      window.parent.postMessage({ protocol: "aio:plugin@2", kind: "file-drop-subscribe", id: registration.id, enabled: false }, "*");
    };
  };
  const fileDrag = () => {
    if (fileDrop) { window.parent.postMessage({ protocol: "aio:plugin@2", kind: "file-drag", id: fileDrop.id }, "*"); }
  };
  Object.defineProperty(window, "aioPlugin", { value: Object.freeze({ request, json, copy, download, navigate, onNavigationChange, deviceView, onFileDrop, fileDrag }), writable: false, configurable: false });
})();
