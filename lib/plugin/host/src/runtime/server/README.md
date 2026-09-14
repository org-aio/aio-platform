# 服务端插件生命周期

`mod.rs` 装配状态；`repository.rs` 负责 Git 发现与版本缓存，`package_repository.rs` 负责二进制包缓存，`package_store.rs` 保存 PostgreSQL 包与不可变版本，`activation_store.rs` 负责激活事务，`store.rs` 管理其余生命周期状态，`marketplace_store.rs` 管理市场索引，`publication.rs` 和 `routes.rs` 提供 HTTP 边界。

市场请求只从 PostgreSQL 缓存读取。远程 HTTPS/Git registry 在启动和请求后异步刷新；刷新失败只保留最近成功索引并写入同步状态，不影响市场页面、已安装插件或租户组合。

市场安装先按来源与可选 revision 查询已发布元数据。二进制包的 SHA-256 版本从 PostgreSQL 恢复缓存并重新校验，不访问远程 Git；不存在的包摘要不能当成 Git ref。旧 Git 安装仍锁定完整提交，不与包内容版本混用。

二进制协议直接迁移到格式 2，旧开发包需要重新打包，不提供格式 1 兼容解码。`plugin.frontend` 的静态资产与后端产物同包保存、校验和激活；`PageDefinition` 只声明入口路径，不保存渲染结果。

`frontend_routes.rs` 提供前端挂载、只读资产、服务桥和卸载接口。`POST /api/runtime/frontend/mount` 接收 `page_id` 并返回 `data: {token, src, revision}`；`POST /api/runtime/frontend/<token>/request` 接收 `method/path/query/body` 并返回 `data: {status, content_type, body}`；`DELETE /api/runtime/frontend/<token>` 释放挂载。服务调用仍要求正常会话 Cookie，只读资产凭证不能替代登录。

`POST /api/runtime/frontend/<token>/renew` 在会话、租户、权限和活动版本校验后续期，不能复活已失效票据。前端每 60 秒续期，页面重新显示时也续期；普通菜单切换不发送 DELETE。资源使用私有 ETag 条件缓存，返回 304 前仍校验权限和文件摘要，入口 HTML 不缓存。`frontend_package.rs` 最多复用 64 份已验证包元数据，避免每次挂载读取 PostgreSQL 整包、解码和重新准备；不缓存授权决策，资产丢失时重新准备。

沙箱 SDK 提供 `aioPlugin.visible` 和 `aioPlugin.onVisibilityChange(listener)`（返回取消订阅函数），插件可据此暂停后台绘制、轮询和订阅。宿主只通知显隐，不擅自冻结插件业务；隐藏页面仍受请求配额和授权约束。没有续期超过 30 分钟后票据失效，再次显示会提示重新打开。

前端文档使用不含 `allow-same-origin` 的 CSP sandbox，只能取自身版本的资产。宿主 `frame-src 'self'` 阻止隔离文档导航到外部站点。每次取文件和调用服务都会重新检查原会话、租户、页面权限、来源、版本及激活代次；停用后再启用也不能复活旧挂载。资产摘要逐次校验，服务调用复用 `service_dispatch.rs` 并与生命周期切换共用锁。`HostConfig.public_origin` 必须与实际浏览器宿主地址一致；本地验证允许 loopback HTTP。宿主最多 256 个挂载、每用户 16 个、有效期 30 分钟及 32 个并发前端请求。

`POST /api/runtime/plugins/publish` 直接接收 `application/vnd.aio.plugin+gzip` 包字节，编解码和完整性验证统一复用 `az-plugin-package`。不接收旧 GitProof JSON，不要求 Actions 或已提交产物。完整包保存于 `plugin_packages.archive`，同来源同 SemVer 不可覆盖不同内容；后台任务通过协议验证和健康检查后原子激活，只有成功的版本才更新市场并允许下载。发布凭证仍绑定租户和 Git 来源，只能由 `AIO_PLUGIN_PUBLISH_ACCOUNTS` 明确授权的管理员创建和撤销。

`GET /api/runtime/packages/<SHA-256>` 为已登录用户下载已成功激活的包。安装、启用、回滚和实例恢复可从数据库重建丢失缓存。发布最多两个并发操作，二进制中心默认 1024 个版本、2 GiB 存储；可通过 `AIO_PLUGIN_PACKAGE_MAX_REVISIONS` 和 `AIO_PLUGIN_PACKAGE_MAX_BYTES` 调整，配额在 PostgreSQL 事务锁内检查。

鉴权和并发检查位于读取上传正文之前。发布请求具有单调递增顺序，激活事务与新请求入队共享租户/来源锁，已被后续请求替代的旧任务不能覆盖活动版本。租户安装或回滚旧包不修改官方市场的当前发布条目。

宿主启动时从 PostgreSQL 的活动租户组合生成 process 实例白名单，并调用监督器清理不在白名单内的受管容器和隔离网络；数据库中的孤立 active 记录同时转为 stopped。

远程 Git 和 HTTPS registry 仅允许标准 443 端口的公网目标，解析出的地址会固定到实际连接并禁用重定向。单次仓库展开、条目数、市场索引响应以及整个版本缓存分别受配额限制；252 默认最多保留 1 GiB、10 万个文件系统条目和 512 个版本，可通过对应的 `AIO_PLUGIN_CACHE_MAX_*` 环境变量收紧。

页面动作统一进入 `POST /api/runtime/pages/action`。Wasm Component 复用 `handle`；process 插件通过仅宿主可访问的 `POST /aio/action` 接收同一事件。该内部路径不属于插件公开 `routes`，不能由浏览器经服务代理直接调用。

## 集成测试

静态产物、离线包、Wasm `definition` 和 process `/aio/definition` 统一调用 `az-plugin-manifest::parse_page_definitions`，接收场景 `children` 树并展开为页面列表后提交。数据库、页面权限和跨插件菜单合并继续使用 `PageDefinition`；目录不承载权限，权限必须写在叶子上。`loads_tree_definition_over_http_and_preserves_leaf_permission` 使用本地 HTTP 代理验证真实入站响应解析，不连接真实容器。

常规运行 `cargo test --no-default-features --features server`。包存储测试显式要求 `AIO_TEST_DATABASE_URL` 指向测试 PostgreSQL，运行 `stores_immutable_binary_packages_and_restores_deleted_cache -- --ignored`；它在临时 schema 中验证版本不可变、二进制持久化和缓存恢复。

HTTP 联调运行 `binary_cli_publishes_downloads_recovers_and_rolls_back_over_http -- --ignored`，需要同值的 `AIO_TEST_DATABASE_URL` 与 `AIO_DATABASE_URL`、`AIO_TEST_CLI` 指向已构建的 CLI，以及测试用 `AIO_BOOTSTRAP_ACCOUNT`、`AIO_BOOTSTRAP_PASSWORD`、`AIO_PLUGIN_PUBLISH_ACCOUNTS`。只使用独立可丢弃的测试数据库。该测试通过真实 CLI 和 HTTP 验证无 Git 目录打包、上传、来源权限、版本冲突、下载、卸载重装、回滚、无效包保留活动版本以及租户切换；不依赖 Docker，也不作为 process 隔离验证的替代。

`frontend_http_mounts_verified_assets_and_revokes_live_access -- --ignored` 使用同一测试数据库和启动账号，经真实 HTTP 验证挂载、CSP、声明外文件拒绝、资产篡改拒绝、权限撤销、启停、更新、租户切换、卸载及退出失效。它直接设置静态包活动绑定，不代替 CLI 发布与真实 Dioxus 浏览器验收。
