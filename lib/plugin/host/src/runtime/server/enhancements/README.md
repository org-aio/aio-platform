# 宿主脚本增强

`controller.rs` 是管理员 API，`store.rs` 统一负责候选验证、租户版本、激活和执行。`EnhancementServicePlugin` 初始化服务并注入同一个 `ScriptRuntime`；WIT、process broker、HTTP、MCP 和 gRPC 都使用它。PostgreSQL 是活动版本的权威源；内存只缓存已验证的不可变代码。每次调用读取一次活动版本，随后完整使用该版本。切换前已接收的调用可以完成，响应返回实际执行的 revision。

只有当前租户的 `plugin:manage` 会话能够保存、查看和激活版本，请求不能指定其他租户。插件只获得清单声明的版本路径调用权，不能发布或激活脚本。以下 API 由 `EnhancementsPlugin` 提供，需要数据库与脚本执行器，不需要配置 Component 存储：

| API | 行为 |
| --- | --- |
| `GET /api/runtime/enhancements` | 当前 revision 和 generation |
| `GET /api/runtime/enhancements/revisions` | 最近 50 个候选版本，时间为毫秒 |
| `POST /api/runtime/enhancements/revisions` | 编译并执行包内样例，成功后保存候选 |
| `GET /api/runtime/enhancements/revisions/{revision}` | 当前租户的完整候选包 |
| `PUT /api/runtime/enhancements/active` | `{ "revision": "摘要", "expected_generation": 0 }` 激活；revision 为 null 时停用 |
| `POST /api/runtime/enhancements/rollback` | 同样的请求结构，仅允许回到本租户曾激活的版本 |

普通升级必须保留已有路径和样例，并用候选实现重新执行旧样例。回滚是显式恢复历史版本，可能使新版本新增的能力暂不可用。版本指针和审计事件在同一事务提交；代数冲突返回 HTTP 409，不能用重试覆盖别人的发布。重新创建宿主后从数据库恢复，执行前仍需重新编译和验证。

每个宿主实例最多同时进行 4 个增强编译、验证或执行任务。任务进入 blocking pool 前就取得配额；等待方取消后，执行中的调用链收到取消信号，配额随实际工作线程结束才释放。代码缓存最多 16 个包，无租户业务状态。

Rhai 和 TypeScript 通过 `call_enhancement(path, input)` 组合包内增强；声明、授权与预算见[增强执行器](../../../../../enhancement/README.md)。服务把原始调用者的能力清单传入整条调用链，不给子调用追加管理员权限，也不为每一层重新申请并发槽或读取活动版本。升级增加依赖不会自动给旧插件增加权限。

测试使用独立本机数据库：

```sh
AIO_ENHANCEMENT_TEST_DATABASE_URL=postgresql://test@127.0.0.1:5432/kernel_test \
  cargo test -p az-plugin-host --features server enhancements::tests -- --ignored
```

## 宿主能力与脚本自举

从 `capabilities.rs` 的 `CapabilityProvider` 看扩展契约，再看 `plugins/worker_capabilities.rs` 的设备服务接入。提供者使用 Dill 与具体 TypeId 注册；重复类型、路径冲突和非法版本路径在启动时失败。目录及 schema 在校验后固定，同一请求不会因重新生成目录而切换实现。提供者的 Future 必须响应丢弃，不能创建无人管理的后台副作用；长期工作应交给已有任务服务。

发布者可通过 `GET /api/runtime/capabilities` 查询路径、说明、输入输出 JSON Schema 与 `read` / `idempotent_write` 类型。该只读目录不授予执行权。默认设备能力为 `/v1/workers/list`、`/v1/workers/tasks/submit`、`/v1/workers/tasks/get`、`/v1/workers/tasks/cancel`，复用同一 `WorkerService`，可省略 `WorkerCapabilitiesPlugin` 或绑定替代提供者。未装配所需能力时，候选可保存但不能激活，在线工具目录也隐藏不可执行的增强。

MCP/gRPC 用户必须同时拥有 `enhancement:<增强路径>`（或已有增强管理权限）及每项 `runtime:<宿主能力路径>`。`plugin:manage` 不授予 runtime 权限。Wasm 插件声明 `[plugin.capabilities].runtime`，与交互用户的 runtime 权限求交集；交互请求结束、成员或会话撤销后拒绝调用。Sidecar 保存进程创建时的 runtime 能力快照；交互处理时向 `/enhancements/invoke` 回传宿主下发的 `x-aio-context`，运行时验证请求来源、租户和有效期，再与用户的显式 runtime 权限求交集。跨插件调用派生绑定目标来源的新上下文，上游请求结束即失效。不带上下文的后台调用只有租户和来源身份，不能调用要求交互会话的设备能力；无效上下文直接拒绝，不回退为后台调用。脚本输入或 `x-aio-user-id` 不能补出用户身份。使用新能力的整包应要求 `host_version = ">=2026.10.16"`。

`protocols::tests::capabilities` 使用真实 PostgreSQL、MCP SDK 和 gRPC 客户端验收样例无副作用、跨语言设备调用、幂等提交、未知字段拒绝、租户隔离、升级回滚、权限撤销、移除插件及取消异步等待。Sidecar broker 与真实 Wasm WIT 用例另行验证能力快照及撤权。通用外部服务导入和 AI 模型生成策略尚未实现。
