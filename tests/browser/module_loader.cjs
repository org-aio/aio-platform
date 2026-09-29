const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { readFile } = require('node:fs/promises');
const { chromium } = require('playwright');

const modules = readFile('lib/plugin/host/src/runtime/server/vendor/es-module-shims.js');
const requested = [];

const server = createServer(async (request, response) => {
  try {
    const path = new URL(request.url, 'http://127.0.0.1').pathname;
    requested.push(path);
    if (path === '/') {
      response.writeHead(200, { 'content-type': 'text/html' });
      return response.end('<!doctype html><html><head><script>globalThis.esmsInitOptions={shimMode:true,nativePassthrough:false}</script><script src="/__aio_modules.js"></script><script type="module-shim" src="/app.js"></script></head><body></body></html>');
    }
    if (path === '/__aio_modules.js') {
      response.writeHead(200, { 'content-type': 'application/javascript' });
      return response.end(await modules);
    }
    if (path === '/app.js') {
      response.writeHead(200, { 'content-type': 'text/javascript' });
      return response.end('globalThis.moduleLoaded = true;');
    }
    response.writeHead(404);
    response.end();
  } catch (error) {
    response.writeHead(500);
    response.end(error.message);
  }
});

(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => globalThis.moduleLoaded === true);
    assert(!requested.some(path => path.endsWith('-typescript.js')), 'JavaScript 模块不得触发 TypeScript 转换器');
  } finally {
    await browser.close();
  }
})().then(() => server.close()).catch(error => {
  console.error(error);
  server.close();
  process.exitCode = 1;
});
