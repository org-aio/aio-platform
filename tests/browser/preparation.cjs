const assert = require('node:assert/strict');
const { mkdir, writeFile } = require('node:fs/promises');
const { resolve } = require('node:path');
const { chromium } = require('playwright');
const { PNG } = require('pngjs');
const { startFixture } = require('./keepalive.cjs');

const component = process.argv.includes('--component');
const output = resolve('target/preparation-test', component ? 'component' : '.');
async function run(browser, server, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const scene = label => page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: label, exact: true }).click();
  const iframe = page.locator('iframe[title="Compose 保活"]');
  const frame = page.frameLocator('iframe[title="Compose 保活"]');
  const measurements = [];
  try {
    for (const mode of ['first', 'refresh']) {
      const before = { ...server.counts };
      const started = Date.now();
      await page.goto(server.origin);
      await page.locator('.application-shell:visible').waitFor();
      assert.equal(await page.locator('.application-shell').evaluate(node => getComputedStyle(node).display), 'grid');
      await page.waitForFunction(() => document.querySelector('iframe[data-aio-prepared="true"]'), null, { timeout: 90000 });
      const preparedMs = Date.now() - started;
      const marker = await frame.locator('body').evaluate(() => window.__preparedMarker = Math.random());
      if (mode === 'first') await page.waitForTimeout(37000);
      assert.equal(server.counts.business, before.business, 'Preparation must not execute business even after the normal request timeout');
      assert.equal(await iframe.evaluate(node => getComputedStyle(node.closest('[data-aio-page]')).visibility), 'hidden');
      const previous = { ...server.counts };
      const opened = Date.now();
      await scene('社区插件');
      await frame.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 1500 });
      assert.equal(await frame.locator('body').evaluate(() => window.__preparedMarker), marker);
      assert.equal(server.counts.mount, previous.mount, 'Showing a prepared page must not mount again');
      await page.waitForFunction(() => document.querySelector('iframe')?.closest('[data-aio-page]')?.dataset.aioPageActive === 'true');
      assert.equal(await iframe.evaluate(node => node.closest('[data-aio-page]').inert), false);
      await page.waitForTimeout(400);
      await frame.getByRole('button', { name: 'Counter', exact: true }).click({ force: true });
      await frame.getByRole('button', { name: '+1', exact: true }).waitFor();
      const interactiveMs = Date.now() - opened;
      assert(interactiveMs < 2000, `Prepared page interaction took ${interactiveMs}ms`);
      await page.waitForTimeout(400);
      const canvas = frame.locator('canvas').first();
      const initial = PNG.sync.read(await canvas.screenshot());
      const button = await frame.getByRole('button', { name: '+1', exact: true }).boundingBox();
      await page.mouse.click(button.x + button.width / 2, button.y + button.height / 2);
      await frame.getByText('1', { exact: true }).waitFor();
      const changed = PNG.sync.read(await canvas.screenshot());
      let pixels = 0;
      for (let i = 0; i < initial.data.length; i += 4) if (initial.data.readUInt32BE(i) !== changed.data.readUInt32BE(i)) pixels++;
      assert(pixels > 30);
      await scene('工作区');
      await scene('社区插件');
      await frame.getByText('1', { exact: true }).waitFor({ timeout: 1000 });
      assert.equal(await frame.locator('body').evaluate(() => window.__preparedMarker), marker);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-${mode}.png`) });
      measurements.push({ mode, preparedMs, interactiveMs, changedCanvasPixels: pixels, backgroundBusiness: 0, remountsOnOpen: 0, retainedState: true });
    }
    assert.deepEqual(errors, []);
    return { viewport: mobile ? 'mobile' : 'desktop', measurements };
  } catch (error) {
    await page.screenshot({ path: resolve(output, 'failure.png') }).catch(() => {});
    console.error({ errors, counts: server.counts });
    throw error;
  } finally { await context.close(); }
}

(async () => {
  await mkdir(output, { recursive: true });
  const server = await startFixture(component ? 2 : 1);
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const results = [];
    for (const mobile of [false, true]) results.push(await run(browser, server, mobile));
    await writeFile(resolve(output, 'report.json'), JSON.stringify(results, null, 2));
    console.log(JSON.stringify(results));
  } finally { await browser.close(); await server.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
