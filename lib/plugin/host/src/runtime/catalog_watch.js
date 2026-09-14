return await new Promise(resolve => {
  const done = event => {
    clearTimeout(timer);
    window.removeEventListener("focus", done);
    window.removeEventListener("aio:catalog-invalidated", done);
    resolve(event?.type === "aio:catalog-invalidated" ? "invalidated" : "poll");
  };
  const timer = setTimeout(done, 30000);
  window.addEventListener("focus", done, { once: true });
  window.addEventListener("aio:catalog-invalidated", done, { once: true });
});
