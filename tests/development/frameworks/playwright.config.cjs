const { defineConfig } = require('@playwright/test');
const { resolve } = require('node:path');
const output = resolve(__dirname, '../../../target/framework-report');
module.exports = defineConfig({
  testDir: __dirname, testMatch: '*.spec.cjs', workers: 1, timeout: 30000,
  outputDir: resolve(output, 'artifacts'),
  reporter: [['list'], ['html', { outputFolder: resolve(output, 'html'), open: 'never' }]],
  use: { channel: 'chrome', headless: true, screenshot: 'only-on-failure', trace: 'retain-on-failure' },
});
