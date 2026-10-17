# 通用插件宿主

`configuration` 和 `identity` 定义装配边界；`runtime/model` 是 HTTP 模型，
`runtime/server` 承载安装、版本、实例、通信桥与交付，`runtime/client`、
`startup` 和 `Workspace` 承载浏览器生命周期。服务端与浏览器以 Cargo feature 分离。

产品负责提供身份实现、品牌、静态贡献、数据库和发布配置。此库不依赖业务插件，
不内置产品域名或仓库所有者。生产应用与本地开发宿主直接消费同一实现。

## 执行器与协议编译选择

`server` 提供宿主契约、Dill 装配、生命周期及持久化服务，不引入内置语言、
Wasm 或 MCP/gRPC 实现。应用在 Cargo 中选取所需实现，再把具体插件交给
`RuntimeState::initialize` 或 `RuntimeHost::initialize`；编译可用不等于已经装配。

| Cargo feature | 装配插件 | 职责 |
| --- | --- | --- |
| `rhai` | `RhaiPlugin` | Rhai 编译与有界执行 |
| `typescript` | `TypeScriptPlugin` | Oxc 转译与 QuickJS 有界执行 |
| `wasm` | `WasmPlugin`、`WasmComponentPlugin` | Wasm 执行 |
| `sidecar` | `ProcessPlugin`、`SidecarPlugin` | 受监督的独立进程 |
| `mcp` | `McpPlugin` | 增强的 MCP Streamable HTTP 入口 |
| `grpc` | `GrpcPlugin` | 增强的 gRPC unary 入口 |
| `remote-mcp` | `RemoteCapabilitiesPlugin` | 导入 MCP HTTP/stdio 工具 |
| `remote-grpc` | `RemoteCapabilitiesPlugin` | 导入描述文件声明的 unary gRPC 方法 |
| `remote` | `RemoteCapabilitiesPlugin` | 同时编译两种出站协议 |

脚本宿主还需装配 `ScriptRuntimePlugin`，需要持久版本时加入
`EnhancementServicePlugin`。协议实现不强制选择语言；例如只编译 `mcp` 的宿主，
可以通过公共 `ScriptExecutor` 接口装配外部执行器。
显式选择脚本运行时却没有执行器时启动失败；不支持的语言与未编译的远端协议会被拒绝。

`default_plugins()` 汇集已编译的默认实现。纯 `server`、`wasm` 或 `sidecar`
组合不要求脚本服务；选择协议的默认组合还需要至少一种内置语言。
完整发布应用显式启用两种语言、两种入站协议和所需执行环境。

组件资源可选依赖增强服务；没有该服务时仍可安装和运行组件，增强调用明确失败。
`RuntimeAssembly::optional_service::<I>()` 声明这种依赖，
`RuntimeState::optional_service::<I>()` 从同一个 Dill 容器解析它。
提供者存在时参与初始化排序和在线重建，重复绑定或构造失败不能被当作缺省值。

依赖边界由 `node scripts/check-host-features.mjs` 检查。
`executor_plugins` 集成测试分别用真实 MCP/gRPC 客户端验证外部执行器的发布与重启恢复；
`component_executors` 在没有内置语言的 `server` 组合中验证整包执行及资源生命周期。

## 已配对设备的应用控制

设备通过现有 AIO 会话配对。macOS 设备升级 worker 后，所属账号在“我的设备”启用应用控制；关闭时取消排队和执行中的应用任务。设备和任务查询均按租户及用户限制，设备令牌与任务租约不返回 Agent。

process 插件声明所需 worker 能力，生产宿主同时以 `AIO_PROCESS_WORKER_CAPABILITIES` 逐项授权；本地项目登记使用独立的 `workspace.manage`，实际项目执行仍由 `workspace.execute` 控制，SSH 主机维护使用 `ssh.manage`，只接受设备端固定动作白名单。插件通过已有 Unix socket 向 `POST /workers` 发送 `x-aio-token`，并提供宿主验证过的 `tenantId`、`userId`。操作分别为 `list`、`openApp`（`workerId`、应用名称 `application`、UUID `requestId`）、`submit` 和 `task`。该接口返回原始 JSON；任务 `state=complete` 才表示客户端成功回报，排队或超时不能推断操作已完成。

宿主检查插件活动版本、已登记用户与有效成员身份。能力只允许打开已安装应用，不接受路径或 shell 命令。应用启动能力的用户范围验证由 worker 的 PostgreSQL 集成测试覆盖。

### 设备访问插件服务

配对、设备身份和租户权限属于宿主公共设施，不依赖智能体安装。客户端携设备凭据主动连接，
不保存 AIO 登录密码、不开放本机监听端口。智能体通过宿主授权能力使用设备；
设备助手和 Skill 同步共用这一条设备连接，业务数据仍由各插件自己的 schema 保存。

`skills.sync` 同时需要进程清单 `worker_capabilities`、宿主环境
`AIO_PROCESS_WORKER_CAPABILITIES=desktop.open-app,skills.sync` 和设备本机 opt-in。
worker 通过 `PUT /api/runtime/workers/self/skills` 的 `{enabled:true}` 开通，
`GET /api/runtime/workers/services` 发现当前租户启用的服务，再以
`POST /api/runtime/workers/services/{source}/skills.sync` 调用固定入口。
宿主每次验证设备未撤销、账号有效、租户安装和活动清单，注入 tenant/user/worker 上下文。
客户端不能指定其他账号、任意服务路径或伪造浏览器会话；网页也不能伪装成设备提交状态。

子插件菜单按已声明父插件共用分组空间；页面、权限、安装状态与数据空间仍保留各自身份。
