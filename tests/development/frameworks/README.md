# 全栈框架浏览器验收

先使用候选 CLI 生成并构建 `next/`、`nuxt/` 两个目录，然后运行：

```sh
cd tests/development
AIO_FRAMEWORK_FIXTURES=/absolute/path/to/projects pnpm exec playwright test --config frameworks/playwright.config.cjs
```

测试启动真实的框架后端 artifact，使用宿主源码中的通信桥、生命周期和 CSP，在隔离 iframe 中验证前端计数、后端计数与租户头，保存桌面及移动截图。这里的 HTTP 宿主是测试夹具，不包含完整应用壳、数据库或 Docker，因此不能作为线上部署证明。
