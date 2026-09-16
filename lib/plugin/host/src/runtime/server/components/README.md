# Component 安装与发布

原生 `aio:plugin@2.0.0` 整包进入独立的持久执行槽。发布校验只更新市场版本，租户安装由用户显式选择；父子依赖按租户校验，不自动安装子插件。页面转换仅用于共享壳导航，业务请求直接使用 v2 二进制契约。

`AIO_COMPONENT_DATABASE_URL` 指向具有创建隔离角色权限的专用 PostgreSQL，数据库必须撤销 PUBLIC 权限。`AIO_COMPONENT_HOME` 是发布目录之外的持久目录，保存 0600 的 `keyring.json` 和 `objects/`；未配置数据库时不开启原生 Component 安装。

安装记录是版本激活的提交点。安装失败恢复上一执行槽，重启时按安装记录恢复执行槽；停用和卸载保留业务 schema、对象及版本历史。声明的业务权限由当前已启用安装版本派生，租户全部成员（包括新加入成员）自动获得，无需配置角色。停用、卸载或版本回退后，下一次请求即按当前版本重新授权。

派生权限使用 `component:<source-id>:<permission>`，组件内仍调用原始权限名，宿主按当前执行来源映射。不同插件声明同名权限不会互相授权，也不能覆盖宿主权限。

前端挂载只返回当前已验证整包的静态资源摘要和大小，不返回后端或迁移文件。入口通过 HTML 解析器注入宿主资源桥和固定模块加载器，让 Compose 的模块、Wasm 与包内 fetch 复用会话缓存；HTML 与模块加载器继续经过实时授权，资源桥不开放额外网络能力。后台预热由宿主目录统一编排。

## 开发装配

本地控制面使用内容摘要验证的源码快照和内存安装绑定，数据库、密钥和对象存储仍复用正式能力实现。
Component 前端单独变化时保留同一执行实例；本地服务由 CLI 启动，宿主准备私有配置、Unix 通信和依赖授权。
沙箱重启重新装载当前运行集合，不向市场版本表写入开发包或伪造提交。

## 权限与菜单验证

`tenant_installation_grants_members_and_menu_visibility_preserves_runtime` 在独立 `component_market_test` PostgreSQL 与真实 WIT 测试组件上验证成员自动授权、跨租户与已移除成员拒绝、旧权限索引回填、升级回退、启停卸载，以及隐藏菜单不改变运行实例和授权。设置 `AIO_COMPONENT_TEST_DATABASE_URL` 与 `AIO_TEST_HEALTHY_COMPONENT` 后运行 `cargo test -p az-plugin-host --features server --lib components:: -- --include-ignored`。

`installation` 统一安装事务、启停卸载与回滚；`delivery` 在同一变更锁内再次检查启用状态、市场版本及排除摘要，自动更新既有安装。回滚仅排除当前市场版本，不永久固定插件；失败沿用安装路径的旧实例恢复。
