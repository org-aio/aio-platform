# MCP 与 gRPC 能力入口

阅读顺序：`access.rs` 的身份与权限 → `mcp.rs` / `grpc.rs` 的协议适配 → `enhancements/store.rs` 的活动版本与执行。适配器使用官方 Rust MCP SDK 与 Tonic，不自行实现 JSON-RPC 握手或 Protobuf 编解码。

`plugins::McpPlugin`、`plugins::GrpcPlugin` 是两个可独立选择的原生宿主插件，都依赖 `EnhancementServicePlugin`。默认组合包含它们；从自定义组合移除后，对应路由返回 404。适配器与现有 HTTP API 共用宿主监听地址，不创建第二个服务器或版本仓库。

精简协议宿主装配 `DatabasePlugin` 与数据库 Provider、`ScriptRuntimePlugin` 与所选语言执行器、`EnhancementServicePlugin`，再选择管理 HTTP、MCP 或 gRPC 路由插件。无需组件存储配置，不创建密钥、Wasm 引擎、Sidecar 或包缓存。Rhai 和 TypeScript 的默认实现分别由 `RhaiPlugin` 与 `TypeScriptPlugin` 提供；应用可通过 `ScriptExecutor` 替换语言实现，所有入口仍复用同一个增强服务。

## 认证与授权

两种协议都从每次请求的认证头调用宿主 `IdentityProvider`，校验成员与会话有效性。租户、用户、来源由宿主确定，消息体不接受这些身份字段。外部调用的来源为 `user:<user_id>`，不会借用某个已安装插件的能力。账户权限中的 `enhancement:/v1/math/double` 只允许对应路径；`plugin:manage` 可执行本租户所有增强。路径没有通配授权，未授权能力不进入工具目录。

Wasm 与 sidecar 继续使用已安装包的 `capabilities.enhancements`，同一 `Enhancements` 服务执行最终能力检查、活动版本选择、执行限额与结果编码。协议层不新增权限数据库。会话撤销在已有连接的下一次请求生效。

## MCP

连接 `https://<host>/api/runtime/mcp`，使用 Streamable HTTP 和宿主认可的认证头。工具名从版本化路径可逆转换：`/v1/math/double` 对应 `v1.math.double`。

```json
{"name":"v1.math.double","arguments":{"input":21}}
```

工具 schema 要求唯一的 `input` 参数，其值是任意 JSON。脚本参数本身尚未声明业务 JSON Schema；TypeScript 类型也不会自动变成运行时校验。结果同时提供 MCP 文本内容和结构化的 `{"revision":"<digest>","output":42}`。

服务不保留认证会话，工具目录按请求读取。激活新版本后客户端重新调用 `tools/list`；调用结果带实际执行版本。当前只提供发现和执行，不提供发布工具、后台任务或主动目录变更通知。Host 必须匹配配置的公开主机，浏览器 Origin 必须匹配公开来源；超过 128 KiB 的请求体（包括分块传输）被拒绝。

## gRPC

使用 HTTP/2，服务为 `aio.runtime.v1.Enhancements`：

- `List(ListRequest)` 返回当前可访问的路径和活动版本。
- `Invoke(InvokeRequest)` 接收 `path` 与 UTF-8 `input_json` 字节，返回 `revision` 与 `output_json`。

跨语言契约位于 `lib/plugin/contract/proto/enhancements.proto`。Rust 客户端启用 `az-plugin-contract/grpc`，从生成的 `grpc::enhancements_client` 导入。认证信息放入请求 metadata；反向代理须转发 HTTP/2 gRPC 和认证头。远程访问由正式入口提供 TLS，本适配器不另开公网端口。

消息最多 65 KiB，脚本输入输出沿用 64 KiB 限制。`grpc-timeout` 在整个请求期间生效，宿主等待上限 30 秒。取消等待不会提早释放仍在工作线程上执行的脚本配额；脚本自身继续受执行时限约束。当前是 unary 调用，不宣称实现流式 RPC。

## 验收与范围

设置本机独立 `AIO_ENHANCEMENT_TEST_DATABASE_URL` 后执行：

```sh
cargo test -p az-plugin-host --features server protocols::tests -- --ignored
```

真实 MCP SDK 与生成的 gRPC 客户端通过同一个 TCP 监听地址验证：权限过滤、匿名与跨租户拒绝、身份不可伪造、Rhai → TypeScript 在线升级、同连接回滚、会话撤销、停用、消息大小、超时和移除插件后的路由消失。测试只装配上述精简组合，并检查组件、密钥、Wasm、Sidecar 和包缓存均不存在。

这是把运行时增强暴露给外部客户端的适配器。连接任意外部 MCP/gRPC 服务并作为入站能力提供者、远程服务注册和流式任务调度仍需各自的生命周期、授权衰减与预算契约。Sidecar 已有独立进程监督器，可以在其内部运行这些协议，但不会因此绕过宿主能力检查。
