const { test, expect } = require('@playwright/test');
const fs = require('node:fs');

// 使用 CLI 已启动的真实沙箱；没有路由拦截、发布或测试激活接口。
const configuration = process.env.AIO_SANDBOX_TARGETS;
if (!configuration) throw new Error('请设置 AIO_SANDBOX_TARGETS，指向真实沙箱 URL 清单');
const targets = JSON.parse(fs.readFileSync(configuration, 'utf8'));
for (const target of targets) {
  for (const mobile of [false, true]) {
    test(`${target.platform} ${target.language} ${mobile ? 'mobile' : 'desktop'}`, async ({ page }, info) => {
      await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 });
      const errors = [], responses = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('response', response => {
        if (response.url().endsWith('/request')) responses.push({ status: response.status(), path: new URL(response.url()).pathname });
      });
      await page.goto(target.url);
      const frame = page.frameLocator('iframe');
      if (target.language === 'kotlin') {
        await frame.getByRole('button', { name: 'Counter', exact: true }).click({ force: true, timeout: 90000 });
        await frame.getByRole('button', { name: '+1', exact: true }).click({ force: true });
        await expect(frame.getByText('1', { exact: true })).toBeVisible();
        await expect.poll(() => responses.some(response => response.status === 200), { timeout: 60000 }).toBe(true);
      } else {
        await frame.getByRole('button', { name: '请求后端 +1', exact: true }).click({ timeout: 90000 });
        await expect.poll(() => responses.some(response => response.status === 200), { timeout: 60000 }).toBe(true);
        const result = target.language === 'rust' ? '服务端结果：1' : '1';
        await expect(frame.getByText(result, { exact: true }).first()).toBeVisible();
      }
      expect(await page.locator('iframe').count()).toBe(1);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      expect(errors).toEqual([]);
      await info.attach('rendered sandbox', { body: await page.screenshot(), contentType: 'image/png' });
      await info.attach('real backend requests', { body: Buffer.from(JSON.stringify({ target, responses, errors }, null, 2)), contentType: 'application/json' });
      await page.goto('about:blank');
    });
  }
}
