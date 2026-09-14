(() => {
  "use strict";
  const pending = new Map();
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  window.addEventListener("message", (event) => {
    const message = event.data;
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
    const id = crypto.randomUUID();
    const timer = setTimeout(() => { pending.delete(id); reject(new Error("Service request timed out")); }, 35000);
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
    const id = crypto.randomUUID();
    const timer = setTimeout(() => { pending.delete(id); reject(new Error("Clipboard request timed out")); }, 5000);
    pending.set(id, { resolve, reject, timer });
    window.parent.postMessage({ protocol: "aio:plugin@2", kind: "clipboard", id, text }, "*");
  });
  Object.defineProperty(window, "aioPlugin", { value: Object.freeze({ request, json, copy }), writable: false, configurable: false });
})();
