# 隔离 Process

模型出站支持 `POST /egress` 转发聊天和 `GET /egress/models` 读取模型列表，两者共用活动版本校验、API 基址授权、密钥转发、并发限制和响应体配额。

v2 包以完整摘要绑定 ELF、前端、迁移和固定运行时镜像。监督器启动无网络、只读根文件系统的容器，宿主通过私有 Unix socket 提供入口、专属 PostgreSQL 通道、模型出站及跨插件调用。

交互调用绑定当前请求上下文；后台只能以已登记且仍属于当前租户的用户身份调用被声明的子插件。停用、卸载及版本变化即时撤销 broker 授权。持久密钥与数据不随容器删除。

宿主与监督器共享 `AIO_PROCESS_ROOT`；`AIO_PROCESS_IMAGES` 为预置镜像的不可变摘要白名单，`AIO_PROCESS_ENDPOINTS` 为允许的 HTTPS 模型 API 基址，均以逗号分隔。未授权镜像或地址的整包不能发布。容器不获得宿主数据库管理员、登录 Cookie 或 Docker socket；数据库登录角色只拥有当前插件与租户 schema 的数据权限。后台服务不能获取用于解密展示的交互请求上下文。

`configuration` 负责凭据、数据库代理和 broker 生命周期；`instance` 负责健康检查与活动实例；`supervision` 只由 root 监督器执行 Docker 操作；`broker` 校验调用身份并流式转发已授权模型请求。安装失败恢复先前启用版本；已停用版本不会被失败重试意外拉起。数据库迁移只允许追加，代码版本回退仍须兼容已应用迁移。
