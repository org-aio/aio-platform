const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require('playwright');

async function run() {
  const base = process.env.AIO_URL;
  assert(base && process.env.AIO_COOKIE_FILE, '需要 AIO_URL 和本次验收的 AIO_COOKIE_FILE');
  const cookies = await fs.readFile(process.env.AIO_COOKIE_FILE, 'utf8');
  const session = cookies.split('\n').map(line => line.split('\t')).find(row => row[5] === 'aio_session')?.[6];
  assert(session, 'Cookie 文件没有 aio_session');
  const output = path.resolve(process.env.AIO_TEST_OUTPUT || 'target/encoding-regression/live');
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const results = [];
  try {
    for (const [name, viewport] of [['desktop', { width: 1440, height: 1000 }], ['mobile', { width: 390, height: 844 }]]) {
      const context = await browser.newContext({ viewport, locale: 'zh-CN' });
      await context.addCookies([{ name: 'aio_session', value: session, url: base, secure: base.startsWith('https:'), httpOnly: true }]);
      const response = await context.request.get(base);
      assert.equal(response.status(), 200);
      assert.match(response.headers()['content-type'], /^text\/html;\s*charset=utf-8$/i);
      const html = await response.body();
      const charsetOffset = html.indexOf('<meta charset="utf-8">');
      const snapshotOffset = html.indexOf('id="aio-startup-snapshot"');
      assert(charsetOffset >= 0 && charsetOffset < 1024 && charsetOffset < snapshotOffset, '字符集声明必须位于首 1024 字节并先于启动快照');
      const { data: catalog } = await (await context.request.get(base + '/api/runtime/catalog')).json();
      const target = catalog.pages.find(page => page.scene.id === 'community');
      assert(target, '验收环境需要一个社区插件');
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(base);
      await page.locator('.application-shell:visible').waitFor({ timeout: 90000 });
      assert.equal(await page.evaluate(() => document.characterSet), 'UTF-8');
      await page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: target.scene.label, exact: true }).click();
      if (name === 'mobile') await page.getByRole('button', { name: '打开菜单', exact: true }).click();
      const navigation = name === 'mobile' ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
      await navigation.getByRole('button', { name: target.label, exact: true }).click();
      const frame = page.frameLocator('iframe[title="' + target.label + '"]');
      await frame.locator('body').waitFor();
      await frame.getByRole('button').first().waitFor({ timeout: 90000 });
      assert(!(await page.locator('.application-shell__header').innerText()).includes('\uFFFD'));
      assert.deepEqual(errors, []);
      await page.screenshot({ path: path.join(output, name + '.png') });
      results.push({ name, encoding: await page.evaluate(() => document.characterSet), contentType: response.headers()['content-type'], charsetOffset, snapshotOffset, scene: target.scene.label, label: target.label, errors });
      await page.goto('about:blank');
      await context.close();
    }
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2));
    console.log(JSON.stringify(results));
  } finally { await browser.close(); }
}
run().catch(error => { console.error(error.message); process.exitCode = 1; });
