# 整包目录持久化

入口是 `plugins::ComponentCatalog`（相当于 Kotlin 的 interface）。`Components` 只持有该接口；正式默认实现是 `PostgresComponentCatalogPlugin`，沿用既有 `component_*` 表，不复制生产数据。

自定义组合省略 `PostgresComponentCatalogPlugin`，在原生插件的 `build` 阶段向同一个 Dill 容器绑定自己的实现：

```rust,ignore
assembly.services
    .add_builder(catalog.clone())
    .bind::<dyn ComponentCatalog, MyCatalog>();
```

保留 `ComponentExecutionPlugin`、`ComponentResourcesPlugin` 和 `ComponentsPlugin`。资源插件用 `require_service` 声明目录依赖；缺失或重复绑定会拒绝装配，不会隐式创建 PostgreSQL 目录。目录的打开、关闭由提供者的生命周期管理；默认实现借用数据库插件的池，不重复关闭。

数据沿 `publish -> install -> restore/handle` 流动。发布保存已验证整包与轻量描述、权限索引；目录和授权查询不加载归档。安装先激活候选，再原子提交安装与首次引导标记，最后切换运行绑定；提交失败恢复旧执行器。恢复逐包加载并检查摘要和来源，缺少执行器时保留安装意图。

实现必须满足以下约束：

- 发布的来源和父归属不可更改；并发发布也必须拒绝父子循环。`replace_publication=false` 用于首次引导，不能覆盖已有最新版本。
- `install` 比较 `expected_generation`；`None` 只接受尚未安装的记录，不能覆盖已有安装。安装和首次引导标记共用提交点。`remove` 也必须比较代数；卸载保留首次引导标记。
- 安装索引按来源、租户、已选摘要一致返回；市场条目的版本是最新发布，安装摘要可以不同。权限索引来自不可变整包，不得在读取失败时当作空集合。
- `process_key` 在并发首次调用中选定一个密钥并始终返回它；进程用户记录按来源和租户隔离。保存的密钥是宿主已经加密的字节。
- `rollouts` 和 `upgrade_target` 保留停用、回滚排除与缺少执行器的选择。执行器缺失不消耗任务；升级前再次核对租户选择。
- 如果声明支持 `delivery_job`，必须与整包发布一起原子完成任务、核对租约和源码摘要；不支持的实现必须明确拒绝，不能先发布再伪造任务成功。

默认 PostgreSQL 实现用事务锁协调来源发布及安装竞争，并在提交内校验版本归属。代数检查保护目录中的安装选择；目录提交与执行器激活仍通过补偿和重启恢复协调，不构成跨后端分布式事务。并发测试验证的是目录提交边界，不能据此宣称多宿主执行实例切换已经具备全局原子性。它仍会读取旧版父插件与交付任务表，因此默认提供者依赖 `StorePlugin`；依赖没有泄漏到 `ComponentResourcesPlugin`。

`tests/component_execution/catalog` 使用文件原子替换实现另一个落盘目录，验证无 PostgreSQL 自举、重启和卸载意图，另用真实 HTTP 验证发布、安装、跨执行器升级、回滚、提交失败补偿、租户隔离及密钥恢复。HTTP 公共管理路由仍需要旧版数据库；测试在独立 schema 中确认没有创建任何整包表。文件实现只是验收夹具，不是生产存储插件，不支持外部构建任务提交。默认 PostgreSQL 另有跨连接的安装竞争、过期删除拒绝和密钥竞争测试。

运行：

```sh
cargo test -p az-plugin-host --features server --test component_executors catalog::file_catalog
AIO_COMPONENT_TEST_DATABASE_URL=postgres://.../component_market_test \
  cargo test -p az-plugin-host --features server --test component_executors catalog:: -- --include-ignored --test-threads=1
```

这次边界不包含 v1 `PluginStore`、市场下架、设备业务表和交付任务持久化，也不移除 host crate 的 PostgreSQL 编译依赖。脚本和整包插件获得统一宿主行为，但不能自行替换内核持久化提供者或绕过身份授权。
