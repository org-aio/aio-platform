# 自动交付

负责仓库发现、持久化构建任务、版本文档和租户升级。构建由独立工作进程完成，宿主只接收已构建包并调用既有运行时验证。

## 原生 v2 交付

`components` 接收经过完整 Git SHA、版本、租约验证的 Bundle，持久化归档再异步调用原生发布验证。发布事务复查最新目标并原子更新任务；不会安装到未选择插件的租户。`component_rollouts` 跟踪既有启用安装的升级结果，回滚通过安装记录的 `excluded_digest` 跳过当前版本，下一版本继续跟进。

`components::delivery_tests` 使用真实 PostgreSQL、HTTP 交付路由及 WIT Component 验证身份拒绝、包元数据绑定、版本替代、多租户升级、停用、卸载、回滚及重复轮询。
