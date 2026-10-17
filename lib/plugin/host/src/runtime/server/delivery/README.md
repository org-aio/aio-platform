# 自动交付

交付协议、文档入口和正式持久化分别装配。构建仍由独立工作进程执行，宿主接收产物后调用既有校验、发布与租户升级链路。

## 装配入口

- `DeliveryPlugin` 只提供构建 HTTP 协议，要求唯一 `DeliveryService` 和共享身份、授权策略。令牌检查、上传大小限制及重试管理权限保留在入口。
- `DocumentationPlugin` 提供版本详情与文档图片，消费可选 `PluginDocumentation`、组件目录和构建历史。普通会话看不到构建历史；未发布的旧版包不返回文档或图片。可以完全不装配构建服务。
- `PostgresDeliveryPlugin` 提供默认 `DeliveryService`，依赖默认目录、管理恢复及组件插件，拥有发现和滚动发布的任务作用域。服务持有当前运行时的弱引用；组合变更时重建，停用时排空任务，不形成容器自引用。
- `PostgresDocumentationPlugin` 提供正式 PostgreSQL 文档后端，依赖 `StorePlugin`。文档按 revision 保存且不可覆盖。

默认组合已同时选择这些插件。自定义宿主直接通过 `assembly.services.add::<YourService>()` 或 `add_builder(...).bind::<dyn DeliveryService, YourService>()` 注册替代实现，再选择 `DeliveryPlugin`；不用选择任何 PostgreSQL 插件。文档同理注册 `dyn PluginDocumentation`。同一具体实现若同时提供目录、交付和文档，必须由同一个插件注册，遵循 TypeId 唯一性。

`contract.rs` 定义构建领取、保留集、退避、续约、上传、完成和重试。替代提供者必须在提交时复核租约与当前目标，并保证发布可见性、故障恢复及版本选择的一致性。发现策略与后台恢复属于提供者插件，通过 `on_ready` 的 `PluginTasks` 管理；协议入口不会偷偷启动默认 GitHub 发现器。通用文档限额和路径检查可调用 `validate_documentation`。

## 默认事务

PostgreSQL 的 v1 包保存与上传状态共用同一事务，失败上传不会占用不可变版本号；发布任务入队与构建状态切换也共用同一事务，提交失败不会泄漏可运行的发布任务。v1/v2 上传与完成在提交前按来源、任务的顺序加锁，再核验租约；过期工作者不能复活任务或覆盖新上传。心跳单条语句原子续约，数据库故障不伪装成租约冲突。

v2 `components` 接收经过完整 Git SHA、版本、租约验证的 Bundle，持久化归档后异步发布。发布事务复查最新目标并原子更新任务；不会安装到未选择插件的租户。`component_rollouts` 跟踪启用安装，回滚通过 `excluded_digest` 跳过当前版本，下一版本继续跟进。

默认实现仍使用跨表 SQL，`StorePlugin` 仍创建与目录事务关联的交付表；这次使整个协调器可替换，没有把默认队列、发现策略或调度时钟逐一拆成独立提供者，也没有移除 host crate 的 PostgreSQL 编译依赖。

## 验收

`tests/legacy_catalog/delivery_http.rs` 用真实文件提交和 HTTP 验证无数据库领取、并发租约、上传、重启恢复、页面发布安装、文档图片、管理权限、提交失败、过期任务与在线替换。文件交付器只接受页面包，不是生产后端，不覆盖跨进程文件并发。

`tests/legacy_catalog/delivery_postgres.rs` 用真实 PostgreSQL 和 HTTP 验证默认提供者、提交触发器故障、无泄漏发布任务、过期/替代租约和连接池关闭错误。`delivery::tests` 验证租约恢复及不可变文档；`components::delivery_tests` 验证真实 WIT 整包的发布与租户选择。
