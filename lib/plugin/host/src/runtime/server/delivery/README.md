# 自动交付

交付协议、文档入口和正式持久化分别装配。构建仍由独立工作进程执行，宿主接收产物后调用既有校验、发布与租户升级链路。

## 装配入口

- `DeliveryPlugin` 只提供构建 HTTP 协议，要求唯一 `DeliveryService` 和共享身份、授权策略。令牌检查、上传大小限制及重试管理权限保留在入口。
- `DocumentationPlugin` 提供版本详情与文档图片，消费可选 `PluginDocumentation`、组件目录和构建历史。普通会话看不到构建历史；未发布的旧版包不返回文档或图片。可以完全不装配构建服务。
- `PostgresDeliveryPlugin` 提供默认 `DeliveryService`，依赖默认目录、管理恢复及组件插件，提供 `DeliverySources`，拥有构建完成后的发布推进任务。服务持有当前运行时的弱引用；组合变更时重建，停用时排空任务，不形成容器自引用。
- `GithubDeliveryDiscoveryPlugin` 只依赖 `DeliverySources`，负责 GitHub 仓库扫描和提交轮询；无需身份、数据库、构建 HTTP、解释器或执行器。默认 API 为公共 GitHub，部署方可指定企业 API 基础地址。
- `PostgresDeliveryRolloutPlugin` 独立拥有租户自动升级与旧包清理。关闭它后，新包仍可发布，但已有租户版本不会自动跟进。
- `PostgresDocumentationPlugin` 提供正式 PostgreSQL 文档后端，依赖 `StorePlugin`。文档按 revision 保存且不可覆盖。

默认组合已同时选择这些插件。自定义宿主直接通过 `assembly.services.add::<YourService>()` 或 `add_builder(...).bind::<dyn DeliveryService, YourService>()` 注册替代实现，再选择 `DeliveryPlugin`；不用选择任何 PostgreSQL 插件。文档同理注册 `dyn PluginDocumentation`。同一具体实现若同时提供目录、交付和文档，必须由同一个插件注册，遵循 TypeId 唯一性。

`contract.rs` 定义构建领取、保留集、退避、续约、上传、完成和重试。替代提供者必须在提交时复核租约与当前目标，并保证发布可见性、故障恢复及版本选择的一致性。发现、发布推进与自动升级分别通过各自 `on_ready` 的 `PluginTasks` 管理；协议入口不会偷偷启动默认 GitHub 发现器。通用文档限额和路径检查可调用 `validate_documentation`。

## 默认事务

PostgreSQL 的 v1 包保存与上传状态共用同一事务，失败上传不会占用不可变版本号；发布任务入队与构建状态切换也共用同一事务，提交失败不会泄漏可运行的发布任务。v1/v2 上传与完成在提交前按来源、任务的顺序加锁，再核验租约；过期工作者不能复活任务或覆盖新上传。心跳单条语句原子续约，数据库故障不伪装成租约冲突。

v2 `components` 接收经过完整 Git SHA、版本、租约验证的 Bundle，持久化归档后异步发布。发布事务复查最新目标并原子更新任务；不会安装到未选择插件的租户。`component_rollouts` 跟踪启用安装，回滚通过 `excluded_digest` 跳过当前版本，下一版本继续跟进。

## 来源与发现插件

`DeliverySources::get` 读取来源代数，发现插件在网络访问前保存该代数，然后调用 `update(git, expected_generation, Some(target))` 原子切换目标并入队。首次读取不存在时用 `None` 代数；传 `None` 目标表示撤销。提交返回 `false` 表示观察已经过时，必须重新读取，不能盲目重试旧目标。GitLab、本地 Git 或 webhook 插件可实现自己的观察逻辑，复用此契约，无需替换构建 HTTP 或 PostgreSQL 队列。本次没有提供这些额外发现适配器。

PostgreSQL 按来源串行提交，更新代数、取消旧租约与入队共用事务；撤销保留墓碑，阻止慢请求复活来源。被撤销但尚未完成的同一源码可显式重新入队，不能恢复旧租约。来源接口属于部署方可信插件边界，不是任意用户的构建命令入口。

GitHub 发现器只轮询自身 API 与 owner 下的来源，不干涉其他发现器的仓库。清单始终按完整提交 SHA 读取；清单被删除或仓库归档会撤销来源，网络错误保留旧目标。分页 ETag 不妨碍已登记来源轮询；请求有响应体上限、超时、并发上限和取消。移除插件会取消网络等待，已开始的存储提交先完成再排空；替换来源提供者会重新装配发现器并清空其缓存。

默认组合显式选择三个插件：`PostgresDeliveryPlugin`、`GithubDeliveryDiscoveryPlugin`、`PostgresDeliveryRolloutPlugin`。`HostConfig.delivery` 仍是默认后台任务开关与 GitHub 扫描参数；自定义组合可只选择发布推进、不选择发现或自动升级。

默认实现仍使用跨表 SQL，`StorePlugin` 仍创建与目录事务关联的交付表。队列租约、退避规则和默认升级策略的定时间隔尚未分别抽成独立提供者，host crate 的 PostgreSQL 编译依赖也仍存在。

## 验收

`tests/legacy_catalog/delivery_http.rs` 用真实文件提交和 HTTP 验证无数据库领取、并发租约、上传、重启恢复、页面发布安装、文档图片、管理权限、提交失败、过期任务与在线替换。文件交付器只接受页面包，不是生产后端，不覆盖跨进程文件并发。

`tests/legacy_catalog/delivery_postgres.rs` 用真实 PostgreSQL 和 HTTP 验证默认提供者、提交触发器故障、无泄漏发布任务、过期/替代租约和连接池关闭错误。`delivery::tests` 验证租约恢复及不可变文档；`components::delivery_tests` 验证真实 WIT 整包的发布与租户选择。

`tests/legacy_catalog/delivery_discovery.rs` 用真实 HTTP GitHub 替身和文件队列验证无数据库运行、分页缓存后的新提交、远端错误、删除清单、陈旧响应、取消、在线卸载、替换存储以及重启后的构建完成。文件夹具不是生产提供者。

`delivery_sources.rs` 验证 PostgreSQL 并发首次发现、跨提供者来源、撤销与重启墓碑、失败事务无泄漏、同一源码重新入队和旧租约拒绝。`delivery_rollout.rs` 经过正式 HTTP 完成 v2 整包上传与后台发布，在线启停升级插件，验证新版本可发布而租户保留旧版，重新启用后才跟进。
