# MCP 与 gRPC 能力入口

阅读顺序：`access.rs` 的身份与权限 → `mcp.rs` / `grpc.rs` 的协议适配 → `capabilities/invocation.rs` 的原生调用或 `enhancements/store.rs` 的脚本版本执行。适配器使用官方 Rust MCP SDK 与 Tonic，不自行实现 JSON-RPC 握手或 Protobuf 编解码。

`plugins::McpPlugin`、`plugins::GrpcPlugin` 是两个可独立选择的原生宿主插件，都依赖 `CapabilityServicePlugin` 和身份提供者，按可选服务依赖观察 `Enhancements`。增强服务增删或重建时，协议路由随同更新。分别启用 `mcp`、`grpc` Cargo feature 后，默认组合才包含对应插件；从自定义组合移除后，对应路由返回 404。适配器与现有 HTTP API 共用宿主监听地址，不创建第二个服务器或版本仓库。

原生协议宿主只需装配 `CapabilityServicePlugin`、能力提供者、身份提供者，再选择 `McpPlugin` 或 `GrpcPlugin`，可以使用无效数据库配置启动。要暴露脚本增强时，再装配 `EnhancementCatalog` 提供者、`ScriptRuntimePlugin` 与所选语言执行器、`EnhancementServicePlugin`；默认持久化由 `PostgresEnhancementCatalogPlugin`、`DatabasePlugin` 与数据库 Provider 提供；管理 HTTP 插件单独选择。无需组件存储配置，不创建密钥、Wasm 引擎、Sidecar 或包缓存。Rhai 和 TypeScript 的默认实现分别由 `RhaiPlugin` 与 `TypeScriptPlugin` 提供；应用可通过 `ScriptExecutor` 替换语言实现，所有入口仍复用同一个增强服务。

## 认证与授权

两种协议都从每次请求的认证头调用宿主 `IdentityProvider`，校验成员与会话有效性。租户、用户、来源由宿主确定，消息体不接受这些身份字段。外部调用的来源为 `user:<user_id>`，不会借用某个已安装插件的能力。账户权限中的 `enhancement:/v1/math/double` 只允许对应增强；`plugin:manage` 可执行本租户所有增强。原生能力逐项要求 `runtime:<path>`，`plugin:manage` 不授予原生能力。发现与直接执行都检查这些权限及租户可用性。路径没有通配授权，未授权能力不进入工具目录。

Wasm 与 sidecar 继续使用已安装包的 `capabilities.enhancements`，同一 `Enhancements` 服务执行最终能力检查、活动版本选择、执行限额与结果编码。协议层不新增权限数据库。会话撤销在已有连接的下一次请求生效。组合增强要求入口和全部传递依赖均获授权；目录隐藏缺少依赖权限的入口，直接调用也被拒绝。`plugin:manage` 的权限仅来自当前用户会话，脚本不能借用它或篡改调用身份。

## MCP

连接 `https://<host>/api/runtime/mcp`，使用 Streamable HTTP 和宿主认可的认证头。增强工具名从版本化路径可逆转换：`/v1/math/double` 对应 `v1.math.double`。原生或导入能力使用 `runtime.v1.math.double`，区分独立的 runtime 权限域；增强路径必须以版本号起始，不会与该前缀冲突。

```json
{"name":"v1.math.double","arguments":{"input":21}}
```

工具 schema 要求唯一的 `input` 参数，其值由能力声明的 `input_schema` 约束；增强省略 schema 时接受任意 JSON。增强结果同时提供 MCP 文本内容和结构化的 `{"revision":"<digest>","output":42}`；原生结果是 `{"output":42}`。`outputSchema` 内嵌对应 `output_schema`，增强结果额外声明 revision。原生工具的只读及幂等提示来自能力 effect，最终执行仍由运行时约束。业务 schema 的局部引用在独立资源范围内解析，包装参数不会改变 `$defs` / `$ref` 的含义。TypeScript 类型注解不会自动成为运行时校验。

schema 由公共执行链实际校验，不能依赖客户端自行执行。已激活的同一租户路径不得改变 schema，停用和回滚也不能绕过；契约变化需新增版本路径。单份 schema 最多 16 KiB，采用离线 JSON Schema 2020-12；完整约束见[增强执行器](../../../../../enhancement/README.md)。

服务不保留认证会话，工具目录按请求读取并按工具名排序，每页最多 64 项、256 KiB。客户端使用 `nextCursor` 或 SDK 的 `list_all_tools` 继续读取；游标不携带授权，每页重新认证和过滤。多页发现期间目录可能更新，需完整新快照时重新开始发现。客户端断连会取消正在等待的原生或增强调用。激活新版本后客户端重新调用 `tools/list`；调用结果带实际执行版本。当前只提供发现和执行，不提供发布工具、后台任务或主动目录变更通知。Host 必须匹配配置的公开主机，浏览器 Origin 必须匹配公开来源；超过 128 KiB 的请求体（包括分块传输）被拒绝。

## gRPC

使用 HTTP/2，原生及导入能力由 `aio.runtime.v1.Capabilities` 提供，无需脚本或数据库：

- `List(ListRequest)` 返回当前用户获准使用且租户可用的路径。
- `Describe(DescribeRequest)` 返回路径、描述、副作用类型和输入输出 schema。隐藏或不存在的路径统一返回 NotFound。
- `Invoke(InvokeRequest)` 返回 `output_json`，直接复用原生异步能力调用。

原生契约位于 `lib/plugin/contract/proto/capabilities.proto`，Rust 客户端为 `grpc::capabilities_client::CapabilitiesClient`。请求模型复用现有路径和 JSON 契约；原生版本通过能力路径约定，没有脚本包摘要，不生成虚假的 revision。

装配增强服务时，另由 `aio.runtime.v1.Enhancements` 提供脚本版本语义；未装配时这些路由不存在：

- `List(ListRequest)` 返回当前可访问的路径和活动版本。
- `Describe(DescribeRequest)` 接收 `path`，返回实际目录版本、路径及 UTF-8 `input_schema_json` / `output_schema_json` 字节。隐藏或不存在的路径统一返回 NotFound。
- `Invoke(InvokeRequest)` 接收 `path` 与 UTF-8 `input_json` 字节，返回 `revision` 与 `output_json`。

跨语言契约位于 `lib/plugin/contract/proto/enhancements.proto`。Rust 客户端启用 `az-plugin-contract/grpc`，从生成的 `grpc::enhancements_client` 导入。认证信息放入请求 metadata；反向代理须转发 HTTP/2 gRPC 和认证头。远程访问由正式入口提供 TLS，本适配器不另开公网端口。

请求消息最多 65 KiB，输入输出 JSON 统一限制为 64 KiB。增强响应最多 65 KiB；原生服务的响应上限为 1 MiB，以容纳最多 4096 条路径的发现结果，调用结果仍受 64 KiB 限制。`grpc-timeout` 在整个请求期间生效，并通过宿主内部截止时间传入脚本与外部 gRPC 调用；各层只能缩短剩余时间，宿主等待上限 30 秒。取消等待会通知整条脚本调用链，但不会提早释放仍在工作线程上执行的配额。嵌套调用共享 100 ms 脚本计算时限、30 秒总等待期限、64 次调用与 1 MiB 累计 JSON 传输预算；宿主 I/O 等待不计入脚本计算时间。当前是 unary 调用，不宣称实现流式 RPC。

## 验收与范围

无数据库的原生入口验收分别执行，证明单协议编译和装配都不要求解释器：

```sh
cargo test -p az-plugin-host --no-default-features --features mcp --test capability_protocols
cargo test -p az-plugin-host --no-default-features --features grpc --test capability_protocols
```

覆盖真实客户端的发现、契约、调用、权限衰减、撤权、提供者替换、协议启停、MCP 分页及断连、gRPC 截止时间。设置本机独立 `AIO_ENHANCEMENT_TEST_DATABASE_URL` 后执行脚本及混合组合验收：

```sh
cargo test -p az-plugin-host --features rhai,typescript,mcp,grpc protocols::tests -- --ignored
```

真实 MCP SDK 与生成的 gRPC 客户端通过同一个 TCP 监听地址验证：权限过滤、匿名与跨租户拒绝、身份不可伪造、Rhai → TypeScript 在线升级、同连接回滚、会话撤销、停用、消息大小、超时和移除插件后的路由消失。测试只装配上述精简组合，并检查组件、密钥、Wasm、Sidecar 和包缓存均不存在。生命周期用例还在线卸载脚本服务、全部解释器和数据库，验证同一 MCP/gRPC 连接继续调用原生能力；重新装配后恢复原有增强版本。

`protocols::tests::schemas` 还覆盖带局部引用的业务契约发现、双协议无效输入及输出拒绝、切换语言、历史契约保留、跨租户独立发布和重启恢复。Sidecar broker 与真实 Wasm 夹具分别验证相同的输入输出检查。

这里负责把原生能力与运行时增强暴露给外部客户端。导入外部 MCP 工具和描述文件定义的 unary gRPC 方法由可选的 [RemoteCapabilitiesPlugin](../remote/README.md) 提供，复用同一授权、租户绑定、schema 和取消预算。部署方文件可在线更新绑定，新调用读取新目录，已有脚本持有同一执行快照；流式任务调度仍待实现。Sidecar 已有独立进程监督器，可以在其内部运行这些协议，但不会因此绕过宿主能力检查。

宿主能力依赖参与同一工具目录过滤与调用前检查。用户需要每项 `runtime:<path>` 权限，增强管理权限不能代替它；两种语言通过 `call_capability` 调用已装配服务。原生能力目录、schema、幂等与 Sidecar 身份边界见 [增强服务](../enhancements/README.md#宿主能力与脚本自举)。

Sidecar 交互请求通过回传宿主下发的 `x-aio-context` 使用用户身份，运行时校验来源和请求有效期，并将进程 runtime 能力快照与用户显式授权求交集。后台调用没有交互用户权限。跨插件 broker 为目标派生独立上下文，不允许拿 A 插件的上下文直接冒充 B。
