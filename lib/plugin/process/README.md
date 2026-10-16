# 原生进程 SDK

`az-plugin-process` 供 v2 Sidecar 插件消费。依赖方向是插件 → SDK → `az-plugin-contract`；不依赖运行时、数据库、身份业务或语言引擎。

入口为 `Host::from_env()`。业务读取 `host.configuration` 中的受限资源，以 `host.protect(router)` 包住业务路由，Controller 使用 `Extension<Caller>` 获取已经认证且绑定租户的身份。把 `/health` 与 `/aio/describe` 合并到外层，再调用 `host.serve(router).await`；返回后关闭业务连接池。

宿主负责实时会话、权限和版本校验；SDK 再验证入口令牌、租户绑定、非空用户及上下文。它不根据任意请求头授予角色，也不提供未认证的独立运行模式。Unix socket 和进程隔离仍由现有监督器管理。

验证：`cargo test -p az-plugin-process`。
