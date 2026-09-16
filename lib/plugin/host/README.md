# 通用插件宿主

`configuration` 和 `identity` 定义装配边界；`runtime/model` 是 HTTP 模型，
`runtime/server` 承载安装、版本、实例、通信桥与交付，`runtime/client`、
`startup` 和 `Workspace` 承载浏览器生命周期。服务端与浏览器以 Cargo feature 分离。

产品负责提供身份实现、品牌、静态贡献、数据库和发布配置。此库不依赖业务插件，
不内置产品域名或仓库所有者。生产应用与本地开发宿主直接消费同一实现。

## 已配对设备的应用控制

设备通过现有 AIO 会话配对。macOS 设备升级 worker 后，所属账号在“我的设备”启用应用控制；关闭时取消排队和执行中的应用任务。设备和任务查询均按租户及用户限制，设备令牌与任务租约不返回 Agent。

process 插件声明 `worker_capabilities = ["desktop.open-app"]`，生产宿主同时配置 `AIO_PROCESS_WORKER_CAPABILITIES=desktop.open-app`。插件通过已有 Unix socket 向 `POST /workers` 发送 `x-aio-token`，并提供宿主验证过的 `tenantId`、`userId`。操作分别为 `list`、`openApp`（`workerId`、应用名称 `application`、UUID `requestId`）和 `task`（`taskId`）。该接口返回原始 JSON；任务 `state=complete` 才表示客户端成功回报，排队或超时不能推断应用已打开。

宿主检查插件活动版本、已登记用户与有效成员身份。能力只允许打开已安装应用，不接受路径或 shell 命令。应用启动能力的用户范围验证由 worker 的 PostgreSQL 集成测试覆盖。
