# 配对设备的原生网页视图

入口 `controller.rs`；生命周期与隔离由 `service.rs` / `service_impl.rs` 管理；授权会话存入 `schema.sql`，原生帧和资源只通过实时有界队列传输，不写入数据库。

设备用既有 Bearer 身份调用 `POST /api/runtime/workers/webviews/access`，只开关本人设备的 `codex.web`；`GET /api/runtime/workers/webviews/channel` 是设备主动建立的出站 WebSocket。重复开通不会重复添加能力，重新连接会关闭旧视图，不重放旧消息。

插件须在 Component v2 process 清单声明 `worker_capabilities=["codex.web"]`。`aioPlugin.deviceView` 提供 `list/open/close`：宿主从当前挂载 Cookie、安装版本和授权推导身份，浏览器不提交账号、租户、来源、版本或设备票据。此能力访问用户原生 Codex 会话与工具，安装授权时应按此实际能力审查。

网页通道和资源位于当前 `components/assets/{token}/__device_view/{id}/`。视图按租户、用户、登录 session、插件来源、revision、mount 和设备绑定；每条消息、资源返回和心跳重新验证。网页登录退出、插件停用或版本变化、设备撤销都使旧授权失效。关闭操作仍校验原始归属，无法关闭其他挂载的视图。

网页 frame 在浏览器和设备两端有上限，宿主不解释 Harness 语义。设备负责原生方法白名单、structured clone 校验、资源路径限制和所属窗口清理。资源上限 32 MiB，返回摘要由宿主复验，禁止缓存和 referrer。仅声明设备视图的插件 CSP 允许嵌套 iframe；内层继续 `sandbox=allow-scripts`。

每设备最多 4 个视图，最多 64 个设备 peer，通道队列容量 32；资源请求最多 32 个，30 秒超时。等待视图 90 秒超时，视图 30 分钟后过期需重新连接。断线不保留/重放写请求，设备控制器关闭对应 Native 窗口。反向代理须允许 WebSocket Upgrade。

聚焦测试：`AIO_TEST_DATABASE_URL=... cargo test -p az-plugin-host --features server worker_webview -- --include-ignored`。使用独立测试数据库；用例创建随机 schema 并清理，验证授权、来源和资源边界、帧顺序、资源应答、六种 owner 字段隔离、重复 attach 和撤权。浏览器 fixture 测试位于配套 `aio-plugin-agent-codex/test/browser`；真实 Codex 发送、停止、审批及文件交互需另外验收。
