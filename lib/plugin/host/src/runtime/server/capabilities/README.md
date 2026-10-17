# 独立能力服务

`CapabilityServicePlugin` 在宿主的同一个 Dill 容器中汇集 `CapabilityProvider`，原生身份使用具体 TypeId；版本路径是业务契约，不是第二套插件注册表。提供者可以是原生服务、外部 MCP 工具或 gRPC 方法，服务本身不需要解释器、数据库、密钥或组件环境。

`CapabilitiesPlugin` 是可选 HTTP 目录适配器，提供 `GET /api/runtime/capabilities`，实时检查管理权限、租户成员及会话。它不依赖 `EnhancementServicePlugin`，目录可见性不授予执行权限。原有增强管理 API 继续由 `EnhancementsPlugin` 提供。

原生插件通过公共异步入口消费同一服务：

```rust,ignore
// scope 必须来自宿主已认证的入口，不能直接反序列化不可信请求。
let invocation = state.capability_invocation(&scope, Some(deadline))?;
let output = invocation.invoke("/v1/calculator/double", &input).await?;
```

原生代码属于可信宿主，入口负责建立身份和显式授权；这里继续检查 `scope.grants.runtime`、租户可用性、成员及会话存活。无用户的租户后台调用不能使用要求交互会话的能力。`available` 是同步目录与授权快照查询，不能替代执行时的会话核验。`IdentityProvider` 可以在只使用目录的组合中省略，创建执行链时必须存在。

Rhai/TypeScript 的 `call_capability` 仅在阻塞解释器线程桥接这个异步执行链。能力服务固定提供者和身份快照，所有克隆共用最多 64 次调用、1 MiB 累计输入输出、单值 64 KiB 与最多 30 秒总截止时间；调用者可以指定更短期限。脚本还保留自身的计算、嵌套和总传输预算。预算耗尽取消整条链，捕获错误或克隆不能重置额度。

输入输出与脚本、远端导入复用 `az-plugin-contract::ValueSchema`：离线 JSON Schema 2020-12，单份 16 KiB、32 层、2048 个节点和有界正则。输入在提供者执行前校验，输出在返回前校验；错误不回显业务值。目录装配和动态发布先编译所有契约，失败不提交新目录。修改已存在路径的提供者、schema 或副作用约定需要新版本路径；每个目录实例最多保留 4096 项历史契约。原生组合重建会重新建立目录，跨重建和重启的宿主能力契约历史目前不持久化；增强包自身的持久契约约束仍由增强版本服务提供。

调用取消、超时或 Future 丢弃都会停止等待提供者，不自动重试业务操作。提供者不能另起无人管理的副作用任务；已经发生的外部写入不会被取消自动撤销。长期任务应交给任务服务并以业务标识保证幂等。

能力插件停止或因提供者、身份依赖变化重建时，旧调用链全部失效，保留旧 `RuntimeState` 也不能继续发起调用。执行中的调用由现有 `PluginTasks` 收容；停止先取消并等待调用 Future 退出，再执行提供者的资源关闭钩子。远端配置文件的正常在线更新则发布新的不可变绑定：旧调用链在原期限内使用原绑定，新调用链使用新绑定；配置更新和插件重建分别遵循明确的生命周期。

验收入口：

```sh
cargo test -p az-plugin-host --no-default-features --features server --test capability_plugins
AIO_REMOTE_TEST_CREDENTIAL=remote-test cargo test -p az-plugin-host \
  --no-default-features --features remote --test capability_plugins -- --include-ignored
```

用例覆盖无解释器/数据库启动、HTTP 目录权限、实际异步调用、输入输出 schema、租户和会话、共享预算、失败不重试、超时与 Future 丢弃、提供者增删/替换、身份重建，以及真实 MCP/gRPC 调用和配置更新期间的绑定快照。更多脚本、Sidecar 和 Wasm 的端到端验收仍使用各自现有入口。
