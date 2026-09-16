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

验证：宿主单元测试，以及隔离 PostgreSQL 的 `worker_pairing_tasks_archives_and_revocation_end_to_end`（环境变量 AIO_TEST_DATABASE_URL / AIO_SPACE_TEST_CLI）。后者验证长轮询等待和唤醒、重复领取、完成回执重传、归档恢复、跨用户拒绝和等待中撤权。真实 NAT / 防火墙证据由产品发布验收文档记录。
