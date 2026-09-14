# 通用宿主浏览器验收

在平台仓库根目录执行。Node 需要能解析 Playwright、pngjs、parse5，画布用例使用本机 Chrome。

```sh
node --test tests/browser/asset_cache.cjs tests/browser/asset_preload.cjs tests/browser/asset_bridge.cjs tests/browser/guest-bridge.cjs
cd host && dx build --platform web --release --debug-symbols false && cd ..
AIO_TEST_KMP_FRONTEND=/absolute/path/to/compose/frontend node tests/browser/keepalive.cjs
AIO_TEST_KMP_FRONTEND=/absolute/path/to/compose/frontend node tests/browser/frontend-layout.cjs
AIO_TEST_KMP_FRONTEND=/absolute/path/to/compose/frontend node tests/browser/preparation.cjs --component
```

测试覆盖资源缓存、通信来源隔离、保活、版本替换、会话变化、准备实例及桌面/移动内容区尺寸。结果保存在 `target/` 各测试目录。协议夹具不替代真实数据库和服务端测试。`AIO_TEST_SHELL` 可指定消费同一宿主实现的产品 Web 产物，验证产品装配；默认使用平台自带开发壳。
