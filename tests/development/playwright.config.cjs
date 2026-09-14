const { defineConfig } = require('@playwright/test');
const path = require('node:path');
const output = path.resolve(__dirname, '../../target/development-report');
module.exports = defineConfig({
  testDir: __dirname,
  testMatch: '*.spec.cjs',
  timeout: 120000,
  workers: 1,
  outputDir: path.join(output, 'artifacts'),
  reporter: [['list'], ['html', { outputFolder: path.join(output, 'html'), open: 'never' }], ['json', { outputFile: path.join(output, 'results.json') }]],
  use: { channel: 'chrome', headless: true, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
});
