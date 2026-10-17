# 旧版插件目录

管理、前端、安装恢复与发布任务通过 `RuntimeState` 从 Dill 取得唯一 `dyn PluginCatalog`。目录提供者负责持久化与原子提交，宿主继续负责认证、权限、包验证、执行器准备和失败补偿。

阅读顺序：`mod.rs` 的整体契约 → `contract.rs` 的安装/发布/包/页面状态 → `queries.rs` 的安装决策与导航偏好 → `../plugins/infrastructure.rs` 的默认提供者。

## 装配

外部原生插件实现 `PluginCatalog` 及其职责接口，在 `build` 中注册同一个具体对象：

```rust,ignore
assembly.services
    .add_builder(catalog.clone())
    .bind::<dyn PluginCatalog, MyCatalog>();
```

与 `RepositoryPlugin`、`ManagementPlugin`、身份及授权提供者组合即可提供发布、下载、目录、安装、升级、回滚、停用、卸载和服务调用。需要菜单偏好或前端时再装配 `NavigationPlugin`、`BootstrapPlugin`、`FrontendPlugin`。这些插件声明接口依赖，不依赖 `StorePlugin` 或 `PgPool`；缺失和歧义绑定在启动前拒绝。

`PluginInstallations` 等接口只是同一提供者的职责拆分，不作为独立 Dill 绑定，也不允许跨提供者拼装一笔安装事务。与 Kotlin 的 `interface` 类似，Rust 的实现可以按职责分文件，但宿主只注入组合后的 `PluginCatalog`。

## 必须保留的语义

- 包内容不可变，同来源同版本不能覆盖其他摘要。上传成功不等于可下载：仅通过发布验证的版本进入下载及发布目录。
- 发布凭据绑定租户与 Git 来源，持久化摘要而非明文。任务领取、完成与重启恢复必须持久化；后来的发布请求阻止旧请求覆盖当前版本。
- 激活须原子保存来源、版本、租户绑定、运行实例和审计；发布还必须与市场条目同事务提交。默认实现同时更新交付安装意图，保留发布顺序锁与任务版本复核。
- `upgrade_eligible` 和 `publication_only` 是候选准备阶段的决策查询，不能代替提交时的版本及来源复核。
- 版本、启用意图、回滚排除和菜单隐藏跨重启保留。执行器缺失只让投影显示暂停，不能改写启用意图。
- 页面状态按租户、版本和页面隔离，写入比较代数；停用或切换版本后旧页面不能继续提交。卸载与删除该安装的页面状态同事务执行。
- 下架只阻止新安装，不能丢失已有租户的管理入口。菜单隐藏不撤销服务授权。
- 失败必须返回，不能保留部分提交。宿主据此撤销候选执行器并保留/恢复旧实例。

## 默认实现与剩余边界

`StorePlugin` 仍创建 `PluginStore`，实现上述接口并使用原有 PostgreSQL 表和事务。它不是另一个目录副本；原先的具体方法直接实现接口，不增加逐方法转发壳。

默认 `DeliveryPlugin` 和 `ToolsPlugin` 仍有跨表事务，显式依赖 `StorePlugin`。默认 v2 `PostgresComponentCatalogPlugin` 也保留此依赖；要彻底省略 PostgreSQL，应同时替换相应默认提供者。构建、发布文档和交付后台仍属于默认交付实现，未声称这些功能已能换存储。

管理市场通过可选 `ToolCatalog` 聚合 CLI 工具；未装配工具提供者时不查询其表，新增或移除提供者会参与在线依赖重建。默认实现仍由 `ToolsPlugin` 提供。设备业务路由中仍需 PostgreSQL 的归档及同步查询直接解析业务连接池，不再借用插件目录取得连接。

宿主 `server` feature 仍编译 PostgreSQL 依赖；本契约证明运行时可不装配/连接它，不等同于完成编译依赖拆分。

## 验收

```sh
cargo test -p az-plugin-host --features server --test component_executors legacy_catalog
```

`tests/legacy_catalog/` 是文件持久化契约夹具，使用临时文件原子替换，丢弃实例后重新读盘。真实 HTTP 覆盖发布任务、包下载、安装、版本切换、回滚、停用卸载、导航偏好、凭据撤销、重启恢复、在线替换和失败补偿。数据库地址故意无效，宿主断言没有 `PgPool`、`PluginStore` 或 `ToolCatalog`。Wasm 执行器使用已有有限替身来观测候选清理，不能视为额外生产执行器。

夹具只接受页面及替代 Wasm，不支持进程、默认交付或跨进程文件并发；它不是生产提供者。正式持久化默认仍为 PostgreSQL，既有隔离数据库测试覆盖包不可变性、旧发布拒绝、下架与工具目录。
