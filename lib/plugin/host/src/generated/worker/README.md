# 用户设备与分布式任务

worker 配对复用 AIO 当前登录账号、工作区和成员有效性。设备只持有独立可撤销凭据，不保存账号密码。

入口：controller.rs；协议：model.rs；Dill 服务：service.rs / service_impl.rs；PostgreSQL：schema.sql。浏览器管理入口由 view.rs 注入现有 Workspace。

设备创建十分钟有效的配对请求，浏览器登录后确认设备信息；设备凭据摘要持久化，配对码一次消费。每账号最多 32 台设备，任务只能下发到本人同工作区设备的已声明能力。任务领取使用行锁和独占租约；120 秒租约由设备每 20 秒续约，失联任务标记 interrupted，不自动重放未知副作用。

归档由 AIO 托管 restic REST 仓库及密钥；路径按工作区和用户隔离。worker 通过自己的设备凭据访问，没有额外的账号、SSH 或手工密码配置。AIO_WORKER_STORAGE_DIR 指向宿主归档数据目录。生产必须备份归档目录、worker_vaults 及宿主 keyring。

## 长期通道协议

首次授权和数据通道分离：配对码只在首次授权时使用；设备凭据不随配对码失效，服务端撤销或账号停用后才拒绝请求。
客户端主动发起 HTTPS `POST /api/runtime/workers/claim`，请求 `{request_id: UUID, wait_seconds: 25}`。服务器最长等待 25 秒，期间每秒检查任务与权限；收到任务立即返回，客户端立即续接。客户端额外每 15 秒心跳，不受任务或 Skill 同步耗时影响。
相同 request_id 在有效租约内返回同一任务与租约；任务结束后返回空，不领取其他任务。新轮询生成新 ID。客户端持久化领取 ID、执行前状态和完成结果；丢失回执时重试相同请求，崩溃后的未知副作用不重做。完成接口接受完全相同结果的幂等重传。
连接全部由客户端出站建立，不开放客户端监听端口，不要求公网 IP、端口映射或 WebSocket Upgrade。代理/断网错误可重试，撤权和停用是终止状态；数据库暂时不可用返回 503，不能伪装成设备撤权。

## 工作区执行与取消

本地 CLI 使用设备 `Authorization: Bearer <token>` 调用 `POST /api/runtime/workers/workspaces/access`，请求 `{"enabled":true}` 开通、`{"enabled":false}` 关闭本人设备的 `workspace.execute`。接口不接受设备 ID 或能力名，也不接受浏览器会话或归档 Basic 凭据。关闭在同一事务中取消该设备排队和运行中的工作区任务并清除租约，其他能力不受影响。

工作区任务复用 `worker_tasks`，输入为 `{"action":"describe"}` 或 `{"action":"run","jobs":[...]}`。执行批次包含 1 至 8 个任务，输入序列化后最多 32768 字节。宿主仍每台设备只允许一个领取中的任务，批次内部并行与命令、路径授权由客户端处理。

登录用户可调用 `POST /api/runtime/workers/tasks/{UUID}/cancel`，返回 `{"data":Task}`。仅本人同租户的排队、运行任务转为 `cancelled` 并清除租约；已结束任务返回原结果。设备后续续租和完成回执会被拒绝，本地执行器应在续租失效时终止执行。

进程 broker 的 `POST /workers` 使用原有 `x-aio-token`，`tenantId`、`userId` 仍由宿主核验插件来源和成员有效性。可选 `capability` 默认 `desktop.open-app`，现有 `openApp` 调用保持不变：

- `{"tenantId":"...","userId":"...","operation":"list","capability":"workspace.execute"}` 发现工作区设备；仅 `list` 支持 `capability:"*"`，返回插件已获授权的 `desktop.open-app`、`workspace.execute` 或 `desktop.control` 任一能力设备。
- `{"tenantId":"...","userId":"...","operation":"submit","capability":"workspace.execute","workerId":"...","requestId":"UUID","input":{"action":"describe"}}` 创建任务。`submit` 接受 `workspace.execute` 和 `desktop.control`；重复请求 ID 必须具有相同设备、能力和输入。
- `{"tenantId":"...","userId":"...","operation":"task","capability":"workspace.execute","taskId":"UUID"}` 查询任务；把 `operation` 改成 `cancel` 可取消任务。任务实际能力必须与请求一致，并包含在插件授权中。

部署者仍需显式配置 `AIO_PROCESS_WORKER_CAPABILITIES` 和插件清单授权，开通设备能力不会自动扩充宿主进程授权。

聚焦回归：`AIO_TEST_DATABASE_URL=... cargo test -p az-plugin-host --features server process::workers -- --include-ignored`，覆盖设备开关、取消幂等、租户/用户/来源/能力隔离以及单设备领取约束。使用独立测试库，并按组件运行时要求先执行 `REVOKE ALL ON SCHEMA public FROM PUBLIC`；测试在随机独立 schema 中创建和清理业务表。

验证：宿主单元测试，以及隔离 PostgreSQL 的 `worker_pairing_tasks_archives_and_revocation_end_to_end`（环境变量 AIO_TEST_DATABASE_URL / AIO_SPACE_TEST_CLI）。后者验证长轮询等待和唤醒、重复领取、完成回执重传、归档恢复、跨用户拒绝和等待中撤权。真实 NAT / 防火墙证据由产品发布验收文档记录。

## 原生桌面控制

`POST /api/runtime/workers/desktop/access` 使用设备 Bearer 凭据与 `{"enabled":true|false}` 单独开关 `desktop.control`，不接受浏览器凭据。关闭会取消该能力的未完成任务，其他能力保留。宿主 process 清单及 `AIO_PROCESS_WORKER_CAPABILITIES` 也须显式授予此能力。

桌面 submit 输入固定为 session UUID、action、arguments 和可选 observation UUID。允许列举、观察、激活应用和固定鼠标/键盘动作；输入限额沿用 32768 字节，写操作必须带 observation。worker 再校验本机开关、单会话独占和观察凭据有效性。宿主只路由到用户设备，不操作服务端桌面；图片回执沿原任务结果通道传输。原生权限与应用兼容性由设备实际检查，complete 不等于用户目标已完成。
