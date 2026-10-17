# 配对设备的原生网页视图

入口 `controller.rs`；生命周期与隔离由 `service.rs` / `service_impl.rs` 管理；授权会话存入 `schema.sql`，原生帧和资源只通过实时有界队列传输，不写入数据库。

设备用既有 Bearer 身份调用 `POST /api/runtime/workers/webviews/access`，必须明确提供 `runtime`，只开关该协议声明的本人设备能力；`GET /api/runtime/workers/webviews/channel` 是设备主动建立的出站 WebSocket。重复开通不会重复添加能力，重新连接会关闭旧视图，不重放旧消息。

插件须在 Component v2 process 清单声明 `worker_capabilities=["codex.web"]`。`aioPlugin.deviceView` 提供 `list/open/close`：宿主从当前挂载 Cookie、安装版本和授权推导身份，浏览器不提交账号、租户、来源、版本或设备票据。此能力访问用户原生 Codex 会话与工具，安装授权时应按此实际能力审查。

网页通道和资源位于当前 `components/assets/{token}/__device_view/{id}/`。视图按租户、用户、登录 session、插件来源、revision、mount、设备及完整协议契约绑定；每条消息、资源返回和心跳重新验证。网页登录退出、插件停用或版本变化、设备撤销都使旧授权失效。关闭操作仍校验原始归属，无法关闭其他挂载的视图。

网页 frame 在浏览器和设备两端有上限，宿主不解释 Harness 语义。设备负责原生方法白名单、structured clone 校验、资源路径限制和所属窗口清理。资源上限 32 MiB，返回摘要由宿主复验，禁止缓存和 referrer。仅声明设备视图的插件 CSP 允许嵌套 iframe；内层继续 `sandbox=allow-scripts`。

每设备最多 4 个视图，最多 64 个设备 peer，设备命令队列容量 32。网页各有独立的 256 帧、32 MiB 缓冲；入队不等待网页消费，慢网页超限只关闭自身，其他浏览器的消息及资源回包继续通过同一设备通道。帧消费完成或视图关闭时释放对应内存预算。资源请求最多 32 个，30 秒超时。等待视图 90 秒超时，视图初始有效期 30 分钟。已连接网页每 10 秒在校验登录、插件版本、挂载和设备授权后，将视图与挂载续期 30 分钟；连续 45 秒没有收到网页消息或 Pong 即关闭。等待、已关闭、已过期或已撤权的视图不会续期，资源读取和普通授权检查也不会延长有效期。断线不保留/重放写请求，设备控制器关闭对应 Native 窗口。反向代理须允许 WebSocket Upgrade。

聚焦测试：`AIO_TEST_DATABASE_URL=... cargo test -p az-plugin-host --features server worker_webview -- --include-ignored`。使用独立测试数据库；用例创建随机 schema 并清理，验证授权、来源和资源边界、帧顺序、资源应答、六种 owner 字段隔离、重复 attach、存活续期及失效后禁止续期。浏览器 fixture 测试位于配套 `aio-plugin-agent-codex/test/browser`；真实 Codex 发送、停止、审批及文件交互需另外验收。

## 插件声明设备协议

`WebviewProtocolProvider` 由设备插件通过 Dill 贡献；宿主不再枚举 Codex、dsh 或隧道。提供者以具体类型的 TypeId 注册和去重，`WebviewRuntime.protocol` 只是跨进程消息的协议值。每项契约声明主能力、可选附加能力及可选网页文档策略。协议重复、能力重叠、标识错误或空提供者会使装配失败。

例如 device 的 `DeviceWebviewsPlugin` 声明 `codex/codex.web/codex.cli` 和 `dsh/dsh.web`；`DeviceTunnelPlugin` 独立声明 `tunnel/tunnel.connect`。新原生插件可贡献 `sensor/sensor.stream` 等协议，无需修改宿主分支。设备端须提供对应运行时，打开消息必须明确协议，缺失或未知协议会被拒绝。后续消息按 `sessionId` 分发。

只有声明 `document` 的协议可通过组件设备视图嵌入网页，是否需要 eval 也来自该契约；没有文档声明的隧道只供独立网关使用。多个网页协议同时匹配组件清单时拒绝挂载。会话持久化完整契约，授权、续期和关闭均验证其一致性，旧数据库中缺少契约的临时会话不会继续授权。

Provider 增删会重建消费其聚合注入的路由插件，取消并排空旧设备和浏览器连接；重新装配后由设备重新连接，旧视图不会复活。撤销单个协议的设备权限只关闭该协议的视图，不修改其他协议能力。`live_tests.rs` 使用真实 HTTP/WebSocket/PostgreSQL 覆盖只移除 Provider 后的断线、拒绝新连接及重新安装恢复。

HTTP 网关、服务目录、目标白名单和浏览器入口由 `aio-plugin-device/server` 提供；宿主不读取内网地址，也不维护另一套配对连接。

建议阅读顺序：`model.rs` 的协议和身份、`protocol.rs` 的插件声明校验、`service_impl.rs` 的权限与生命周期、`controller.rs` 的传输和资源 CSP。
