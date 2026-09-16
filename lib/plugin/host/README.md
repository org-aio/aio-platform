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

### 设备访问插件服务

配对、设备身份和租户权限属于宿主公共设施，不依赖智能体安装。客户端携设备凭据主动连接，
不保存 AIO 登录密码、不开放本机监听端口。智能体通过宿主授权能力使用设备；
空间管家和 Skill 同步共用这一条设备连接，业务数据仍由各插件自己的 schema 保存。

`skills.sync` 同时需要进程清单 `worker_capabilities`、宿主环境
`AIO_PROCESS_WORKER_CAPABILITIES=desktop.open-app,skills.sync` 和设备本机 opt-in。
worker 通过 `PUT /api/runtime/workers/self/skills` 的 `{enabled:true}` 开通，
`GET /api/runtime/workers/services` 发现当前租户启用的服务，再以
`POST /api/runtime/workers/services/{source}/skills.sync` 调用固定入口。
宿主每次验证设备未撤销、账号有效、租户安装和活动清单，注入 tenant/user/worker 上下文。
客户端不能指定其他账号、任意服务路径或伪造浏览器会话；网页也不能伪装成设备提交状态。

子插件菜单按已声明父插件共用分组空间；页面、权限、安装状态与数据空间仍保留各自身份。
