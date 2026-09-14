const assert = require('node:assert/strict');
const { mkdir, writeFile } = require('node:fs/promises');
const { resolve } = require('node:path');
const { chromium } = require('playwright');
const { PNG } = require('pngjs');

const output = resolve('target/frontend-layout');
const viewports = [
  ['desktop-tall', { width: 1920, height: 1200 }],
  ['desktop', { width: 1440, height: 1000 }],
  ['mobile', { width: 390, height: 844 }],
  ['mobile-landscape', { width: 844, height: 390 }],
];

async function geometry(iframe) {
  return iframe.evaluate(node => {
    const content = node.closest('.application-shell__content, .application-fullscreen__content');
    const style = getComputedStyle(content);
    const outer = content.getBoundingClientRect();
    const frame = node.getBoundingClientRect();
    return {
      width: frame.width, height: frame.height,
      expectedWidth: content.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
      expectedHeight: content.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom),
      bottomGap: outer.bottom - frame.bottom - parseFloat(style.paddingBottom),
      overflow: document.documentElement.scrollWidth > innerWidth || content.scrollHeight > content.clientHeight + 1,
    };
  });
}

async function run() {
  const { startFixture } = require('./keepalive.cjs');
  await mkdir(output, { recursive: true });
  const server = await startFixture(2);
  server.fixture.catalog.pages.find(page => page.id === '测试工作区').body.content = 'Scrollable native page. '.repeat(10000);
  server.fixture.catalog.pages.find(page => page.id === '独立账户页').body = { kind: 'frontend', entry: 'index.html' };
  server.fixture.catalog.page_versions['独立账户页'] = server.fixture.catalog.page_versions['Compose 保活'];
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const results = [];
  try {
    const page = await browser.newPage({ viewport: viewports[0][1] });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    const scene = label => page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: label, exact: true }).click();
    await page.goto(server.origin);
    await scene('社区插件');
    const iframe = page.locator('iframe[title="Compose 保活"]');
    const frame = page.frameLocator('iframe[title="Compose 保活"]');
    await frame.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 90000 });
    await page.waitForTimeout(400);
    await frame.getByRole('button', { name: 'Counter', exact: true }).click({ force: true });
    await frame.getByRole('button', { name: '+1', exact: true }).waitFor();
    const marker = await frame.locator('body').evaluate(() => window.__layoutMarker = Math.random());
    const mounts = server.counts.mount;
    for (const [name, viewport] of viewports) {
      await page.setViewportSize(viewport);
      await page.waitForTimeout(600);
      const bounds = await geometry(iframe);
      assert(Math.abs(bounds.width - bounds.expectedWidth) <= 1, JSON.stringify(bounds));
      assert(Math.abs(bounds.height - bounds.expectedHeight) <= 1, JSON.stringify(bounds));
      assert(Math.abs(bounds.bottomGap) <= 1, JSON.stringify(bounds));
      assert.equal(bounds.overflow, false, JSON.stringify(bounds));
      assert.equal(await frame.locator('body').evaluate(() => window.__layoutMarker), marker);
      const canvasBounds = await frame.locator('canvas').first().boundingBox();
      assert(Math.abs(canvasBounds.height - bounds.height) <= 1, 'Compose canvas must resize with its iframe');
      const png = PNG.sync.read(await frame.locator('canvas').first().screenshot());
      let colored = 0;
      for (let i = 0; i < png.data.length; i += 4) if (png.data[i + 1] > png.data[i] + 15 && png.data[i + 1] > png.data[i + 2]) colored++;
      assert(colored > 100, 'Counter must remain painted');
      await page.screenshot({ path: resolve(output, `${name}.png`) });
      results.push({ name, viewport, ...bounds, coloredCanvasPixels: colored });
    }
    await page.setViewportSize(viewports[1][1]);
    await scene('工作区');
    const native = page.locator('.application-shell__content:visible');
    assert(await native.evaluate(node => node.scrollHeight > node.clientHeight));
    assert(await native.evaluate(node => { node.scrollTop = node.scrollHeight; return node.scrollTop > 0; }));
    await scene('社区插件');
    await frame.getByRole('button', { name: '+1', exact: true }).waitFor();
    assert.equal(await frame.locator('body').evaluate(() => window.__layoutMarker), marker);
    assert.equal(server.counts.mount, mounts);
    await page.getByRole('button', { name: '收起菜单', exact: true }).click();
    await page.waitForTimeout(400);
    const collapsed = await geometry(iframe);
    assert(Math.abs(collapsed.width - collapsed.expectedWidth) <= 1);
    assert(Math.abs(collapsed.height - collapsed.expectedHeight) <= 1);
    await page.getByRole('button', { name: '展开菜单', exact: true }).click();
    await page.locator('.application-shell__sidebar button[aria-label$="的账户菜单"]').click();
    await page.getByRole('menuitem', { name: '独立账户页', exact: true }).click();
    const account = page.locator('.application-fullscreen:visible iframe');
    await account.waitFor();
    const fullscreen = await geometry(account);
    assert(Math.abs(fullscreen.height - fullscreen.expectedHeight) <= 1, JSON.stringify(fullscreen));
    assert.equal(fullscreen.overflow, false);
    await page.getByRole('button', { name: '返回主后台', exact: true }).click();
    assert.equal(await frame.locator('body').evaluate(() => window.__layoutMarker), marker);
    assert.deepEqual(errors, []);
    const report = { results, collapsed, fullscreen, nativeScroll: true, remountsOnResize: 0, errors };
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally {
    await browser.close();
    await server.close();
  }
}

if (require.main === module) run().catch(error => { console.error(error); process.exitCode = 1; });
module.exports = { geometry, viewports };
