# 个人配置

宿主内独立的多设备个人配置领域，复用账号、租户、worker 配对和撤销。不依赖智能体或模型服务。文件同步由 Space 插件接管，宿主不再提供账号菜单“个人配置”及其编辑、历史、同步设备和资源恢复面板。

## 契约

浏览器前缀 `/api/runtime/personal-config` 使用当前会话；worker 前缀 `/api/runtime/workers/personal-config` 使用设备 Bearer 凭据并要求 `config.sync`。所有者只能由宿主身份推导，每张表和查询均限定 `tenant_id + user_id`。浏览器不能代替设备首次开启同步。

- `GET /catalog`：元数据、库版本、设备报告，无配置正文。
- `POST /entries`：`WriteEntry`，`expected` 为当前版本，新建为 null；版本竞争返回 409，相同正文和元数据可幂等重试。
- `GET /entries/{id}[?revision=N]` 和 `/history`：按需解密当前正文或最近十个历史版本。
- `GET /changes?after=N&wait=25`：出站 HTTPS 长轮询，每秒检测更新、账号和设备权限。
- `POST /report`：设备同步报告，不含正文；`POST /resolve`：网页按双方哈希确认冲突选择。
- `PUT /self` 和 `/devices/{id}`：设备本机启用/停用；网页仅可停用自己的设备。

类型：`file` 文本/JSONC，`env` 单个变量，`paths` PATH 附加目录 JSON 数组，`function` Bash 函数体，`command` macOS bundle ID，`asset` restic 归档引用。函数名遵循 Bash 标识符规则，函数体最多 32 KiB，`format=bash`、`secret=true`、`executable=false`；设备本机用 `bash -n` 校验后生成 Bash 专用脚本。层级：`shared < os:darwin/linux < device:UUID`，最高层完整覆盖同名条目。删除记录保留墓碑，避免离线旧设备复活已删除配置；删除覆盖层代表隐藏该配置，回到共享配置需编辑覆盖项或明确恢复历史版本。

正文及历史正文通过宿主 Keyring 加密，AAD 包含租户、账号、条目。元数据和同步报告可查询，但不得放入密码或正文。每条正文上限 256 KiB、每账号当前配置上限 32 MiB/4096 项。必须连同宿主 keyring 备份数据库；仅有数据库无法解密。`secret` 标记不改变加密策略，所有正文均加密。

## 实现和验收

`service` 为契约，`service_impl` 通过 Dill 获取 PgPool/Keyring；`schema.sql` 由宿主迁移加载。`controller` 负责身份和长轮询传输，保留 Space 客户端使用的同步接口和已有加密数据。

`AIO_TEST_DATABASE_URL` 指向隔离 PostgreSQL、`AIO_SPACE_TEST_CLI` 指向构建后的 CLI，执行：

```sh
cargo test -p az-plugin-host --no-default-features --features server personal_configuration_devices_isolation_revisions_and_sync -- --ignored
cargo check -p az-plugin-host --no-default-features --features web --target wasm32-unknown-unknown
```

集成测试覆盖两个已配对设备、跨租户/账号隔离、密文、并发 CAS、历史读取、实际 CLI 三方合并/冲突解决、环境变量设备覆盖和长轮询停权。产品发布后另做桌面/手机浏览器与当前 Mac 实验；模拟第二设备不等于实体 Mac mini 验收。
