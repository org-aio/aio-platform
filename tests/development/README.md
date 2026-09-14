# 本地沙箱端到端验收

三种插件先分别由 `aio plugin init` 在无 Git 远端的临时目录创建，再通过 `aio plugin dev` 启动。测试读取真实宿主 URL；不模拟业务接口。

```sh
npm ci --prefix tests/development
AIO_SANDBOX_TARGETS=/absolute/path/to/targets.json npm test --prefix tests/development
```

清单格式：`[{"platform":"macos-arm64","language":"kotlin","url":"http://127.0.0.1:4200"}]`。语言为 `kotlin`、`rust` 或 `typescript`；Linux 可通过 SSH 转发仅监听回环的宿主端口。

测试覆盖桌面/移动端真实壳、计数及前后端调用，并保存 Playwright HTML/JSON 报告、截图和失败追踪到 `target/development-report/`。需要本机 Chrome。不要提交含数据库连接或会话凭据的运行目录。
