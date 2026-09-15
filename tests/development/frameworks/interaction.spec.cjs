const { test, expect } = require('@playwright/test');
const { resolve } = require('node:path');
const { start } = require('./host.cjs');
const fixtures = process.env.AIO_FRAMEWORK_FIXTURES;
if (!fixtures) throw new Error('请设置 AIO_FRAMEWORK_FIXTURES，包含已构建的 next/ 和 nuxt/ 项目');

for (const framework of ['next', 'nuxt']) {
  test(`${framework} 静态页面通过真实通信桥调用独立框架后端`, async ({ page }, info) => {
    const host = await start(resolve(fixtures, framework));
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    try {
      await page.goto(host.address);
      const frame = page.frameLocator('iframe');
      await frame.getByRole('button', { name: '前端 +1', exact: true }).click();
      await expect(frame.getByLabel('前端计数', { exact: true })).toHaveText('1');
      await frame.getByRole('button', { name: '后端 +1', exact: true }).click();
      await expect(frame.getByLabel('后端计数', { exact: true })).toHaveText('1');
      await expect(frame.getByRole('status').filter({ hasText: '租户：browser-tenant' })).toBeVisible();
      expect(host.requests).toContainEqual({ path: '/backend/api/counter', status: 200 });
      expect(errors).toEqual([]);
      await info.attach('桌面页面', { body: await page.screenshot(), contentType: 'image/png' });
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(frame.getByRole('button', { name: '后端 +1', exact: true })).toBeVisible();
      await info.attach('移动页面', { body: await page.screenshot(), contentType: 'image/png' });
    } finally { await page.goto('about:blank'); await host.close(); }
  });
}
