# 执行资源与可选 Wasm 宿主

默认构建提供密钥、数据库、对象存储和 `InvocationResources`，不引入 Wasmtime、WASI 或 Cranelift。需要 `ComponentEngine`、执行槽和 WIT 绑定时显式启用 `wasm` feature。宿主服务的请求使用 `az-plugin-contract::transport`；`InvocationResources::validate_scope` 检查资源归属，可信原生执行器可通过 `ScopedDatabase::begin` 使用受限事务。

`ComponentEngine::with_cache` 使用宿主独占的目录保存 Wasmtime 编译缓存，发布校验、安装及重启恢复复用同一份机器码。缓存键包含引擎与编译配置，磁盘软限额为 512 MiB/4096 项；只缓存代码，不缓存租户实例、数据库连接或授权。宿主必须限制目录写权限，不接收插件上传的预编译机器码。

提供按调用上下文隔离的 Component 执行与能力代理，不依赖业务插件。数据库连接必须使用宿主为当前插件和租户配置的专属最小权限角色，不能传入平台管理连接。

每次调用临时持有完整执行状态，只有成功结束才归还实例；取消、超时或陷阱会销毁状态，未提交事务进入回滚。失效实例不能复用。空值参数使用 PostgreSQL 的上下文类型推导，不强制转换为文本。

`cargo test -p az-plugin-runtime --features wasm` 执行无外部依赖的测试。完整测试需要一次性 PostgreSQL（已撤销 public schema 的 PUBLIC 权限）以及已构建的 Kotlin 示例，三个路径/地址通过环境变量指定：

```sh
export AIO_TEST_DATABASE_URL=postgres://test_admin@127.0.0.1:55432/postgres
export AIO_TEST_COMPONENT=/absolute/path/aio-plugin-kmp-example/dist/plugin.wasm
export AIO_TEST_MIGRATION=/absolute/path/aio-plugin-kmp-example/backend/migrations/0001_counter.sql
cargo test -p az-plugin-runtime --features wasm -- --include-ignored
```

这些测试创建随机来源的独立角色和 schema，仅对一次性测试库运行。`DatabaseProvisioner::install` 持久保存加密数据库凭据及有序迁移校验和，支持追加受限建表和索引迁移，拒绝改写、删除或重排历史。公网接入状态由 `docs/refactor/README.md` 跟踪。

## 整包执行槽

`ComponentSlot` 是按来源 UUID 和租户绑定的进程内执行单元。它只接受 `az-plugin-bundle::VerifiedBundle`：校验能力授予、宿主版本、实例描述和前端入口，运行 prepare/health/activate/health，再排空在途请求并一起交换页面资源、描述和实例。失败或准备被取消时保留旧版本。调用必须匹配当前整包摘要及租户；升级后旧摘要失效，停用后拒绝调用。旧实例清理失败只记录日志并销毁，不撤销已激活的新版本。

新旧迁移文件集合必须完全一致，否则拒绝在线替换，交由维护迁移流程。该单元不提供持久化注册表、登录授权或迁移执行器，不应直接暴露成公网安装接口。首次实例化前数据库必须由可信宿主完成受控初始化；停用不删除业务数据。`ReleaseSnapshot` 是只读资源快照，不是认证票据。

Rust 和 Kotlin SDK 的 WASI 仅开放受限 I/O、时间、安全随机及空环境。标准输出和错误输出各有 4 KiB 缓冲，不继承宿主环境、文件预打开目录、终端或网络。

## 持久激活与加密

`PersistentComponentSlot::open` 接收宿主发布记录连接池并创建持久执行槽，在宿主数据库保存整包、授权、活动摘要和激活历史。恢复时重新校验整包和当前宿主授权；发布前校验候选，再以本地锁和 PostgreSQL 行锁排空在途请求，提交活动版本后切换实例。其他宿主上的旧版本调用被数据库记录拒绝。详情见 `src/registry/README.md`。

`Keyring` 以版本化 AES-256-GCM 信封绑定插件、租户和用途，支持保留旧密钥后轮换活动密钥。只有运行阶段且授予 cryptography 的实例可以调用；数据库凭据不进入插件。宿主不得启用包含动态角色密码的 SQL 语句日志。

宿主停止时，持久化槽调用 `unload()` 释放执行实例，保留活动版本；全部槽与后台工作退出后调用 `ComponentDatabase::shutdown()` 关闭所持连接池。`deactivate()` 会改变持久化激活状态，只用于明确停用插件。

## 可替换资源

`ComponentDatabase` 是数据库分配与迁移接口，默认 `DatabaseProvisioner` 直接实现它。替代实现仍以 PostgreSQL 为持久化协议，使用 `ScopedDatabase::connect_options` 创建句柄：运行时检查登录身份与当前角色一致、当前 schema、跨 schema 权限、角色继承及建表权限。`InvocationResources::validate_scope` 再检查来源与租户。Sidecar 从这个已校验句柄导出 `connection_options()`，不会另向 Provider 索取未经检查的凭据。发布记录使用 `release_pool()`，保持原有数据库位置与恢复格式。

`ObjectStorage` 只负责后端读写和关闭，`ObjectStore` 负责来源与租户命名空间、键长度、摘要地址和 16 MiB 读写上限。`ObjectKey` 不接受外部构造，后端消费 `namespace()` 与 `digest()`。默认 `FileObjectStorage` 继续读取既有目录和摘要文件，保留原子写入与符号链接拒绝。

```rust,ignore
let backend = Arc::new(FileObjectStorage::new(root));
let storage = ObjectStore::new(backend, "source", "tenant")?;
storage.write("record", b"data").await?;
```

替代后端属于可信原生代码，其内部 I/O、内存及远端请求仍由实现负责约束；脚本和 Sidecar 不直接取得 Provider。`cargo test -p az-plugin-runtime --test storage` 验证后端替换、租户和来源隔离、超限拒绝、现有文件布局读取及符号链接拒绝。
