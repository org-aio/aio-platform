# 通用宿主浏览器验收

在平台仓库根目录执行。Node 需要能解析 Playwright、pngjs、parse5，画布用例使用本机 Chrome。

```sh
node --test tests/browser/asset_cache.cjs tests/browser/asset_preload.cjs tests/browser/asset_bridge.cjs tests/browser/guest-bridge.cjs
cd host && dx build --platform web --release --debug-symbols false && cd ..
AIO_TEST_KMP_FRONTEND=/absolute/path/to/compose/frontend node tests/browser/keepalive.cjs
AIO_TEST_KMP_FRONTEND=/absolute/path/to/compose/frontend node tests/browser/frontend-layout.cjs
AIO_TEST_KMP_FRONTEND=/absolute/path/to/compose/frontend node tests/browser/preparation.cjs --component
node tests/browser/component-lifecycle.cjs
```

测试覆盖资源缓存、通信来源隔离、保活、版本替换、会话变化、准备实例及桌面/移动内容区尺寸。结果保存在 `target/` 各测试目录。协议夹具不替代真实数据库和服务端测试。`AIO_TEST_SHELL` 可指定消费同一宿主实现的产品 Web 产物，验证产品装配；默认使用平台自带开发壳。

`AIO_URL=https://your-host AIO_COOKIE_FILE=/private/test-cookies.txt node tests/browser/document-encoding.cjs` 使用 Netscape 格式的验收会话文件，检查首页 UTF-8 响应头、前 1024 字节内的字符集声明，以及桌面 / 移动壳中与目录一致的中文场景和插件名称。报告保存在 `target/encoding-regression/live/`；`AIO_TEST_OUTPUT` 可覆盖输出目录。

`component-lifecycle.cjs` 使用 Playwright 自带 Chromium（可用 `AIO_BROWSER_EXECUTABLE` 指定）。真实 opaque iframe 加载正式宿主与 guest SDK，模拟服务端票据 API，验证暂停释放、原实例恢复、过期续期、20 次切换及 bfcache；管理请求独立分发、暂停拒绝、恢复使用新票据。成功写操作等待宿主提交本次请求对应的新目录，实际目录监听器覆盖并发请求、旧通知隔离、刷新失败和暂停取消。CI 自动执行，不替代真实管理权限和产品市场 UI 验收。

资源流验收同时覆盖真实点击触发 100 MiB 下载、页面暂停拒绝、新挂载票据用于后续下载。浏览器票据 API 是夹具；真实身份、插件权限、安装代数与停止中的流由 `component_executors::catalog::streaming` 和 Sidecar Unix 集成测试验证。
