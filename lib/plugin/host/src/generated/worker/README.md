# 用户设备与分布式任务

worker 配对复用 AIO 当前登录账号、工作区和成员有效性。设备只持有独立可撤销凭据，不保存账号密码。

入口：controller.rs；协议：model.rs；Dill 服务：service.rs / service_impl.rs；PostgreSQL：schema.sql。浏览器管理入口由 view.rs 注入现有 Workspace。

设备创建十分钟有效的配对请求，浏览器登录后确认设备信息；设备凭据摘要持久化，配对码一次消费。每账号最多 32 台设备，任务只能下发到本人同工作区设备的已声明能力。任务领取使用行锁和独占租约；120 秒租约由设备每 30 秒续约，失联任务标记 interrupted，不自动重放未知副作用。

归档由 AIO 托管 restic REST 仓库及密钥；路径按工作区和用户隔离。worker 通过自己的设备凭据访问，没有额外的账号、SSH 或手工密码配置。AIO_WORKER_STORAGE_DIR 指向宿主归档数据目录。生产必须备份归档目录、worker_vaults 及宿主 keyring。
