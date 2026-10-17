# 增强存储提供者

从 `EnhancementCatalog` 查看契约，从 `PostgresEnhancementCatalogPlugin` 查看默认装配。`EnhancementServicePlugin` 只要求唯一的存储提供者、脚本运行时、能力服务和身份，不再依赖数据库插件。HTTP、MCP、gRPC、WIT、Sidecar 和演进策略继续消费同一个增强服务。正式默认使用 PostgreSQL；文件实现只在测试目录中用于证明可替换性。

应用可以省略 `PostgresEnhancementCatalogPlugin`、`DatabasePlugin` 和 `PostgresPlugin`，通过自己的原生插件向 `assembly.services` 绑定 `dyn EnhancementCatalog`。提供者必须声明自己的初始化和停止钩子；公共服务通过接口依赖参与启动排序和在线重建。重复绑定、缺失提供者、初始化失败都拒绝装配，不会创建内存后备或悄悄返回空目录。在线换提供者时，旧缓存随增强服务重建，候选与历史迁移仍由部署方负责；替换接口不意味着自动搬迁数据。

存储负责以下事务边界：

- `propose` 按包内容摘要保存不可变候选，重复提交保留原作者和创建时间。
- `activate` 在同一事务中检查预期代数、回滚资格、已发布路径的历史 schema，更新指针并追加事件。代数冲突返回 `None`。停用或回滚不能释放已发布的 schema；历史资格查询不能截断成最近 50 条。
- `CommitAuthorization` 是由宿主创建的延迟复核 Future，必须在写锁等待和校验之后、提交之前执行。拒绝、取消或提交失败均不能留下候选、指针或事件。不得把复核放到锁之前或交给后台任务。
- 观察按租户、激活代数及路径计数，同一摘要重新激活使用新的代数。查询必须验证版本和代数属于当前租户，不保存请求输入、输出或凭据。时间字段统一为 Unix 毫秒。

公共执行链继续负责编译、样例重放、摘要验证、可用语言检查、预算与授权。`Package` 和模型不依赖 SQLx；默认表、SQL 和行解码集中在此目录的 PostgreSQL 实现中。当前宿主 crate 的其他业务仍依赖 SQLx，本次契约拆分不等于整个宿主已不编译 PostgreSQL。

验收入口：

```sh
AIO_ENHANCEMENT_TEST_DATABASE_URL=postgresql://test@127.0.0.1:5432/kernel_test \
  cargo test -p az-plugin-host --features rhai,typescript,mcp,grpc,test-support \
  --test enhancement_catalog -- --include-ignored --test-threads=1
```

文件夹具以无效数据库配置运行真实 HTTP、MCP 和 gRPC 双语言调用，覆盖提交失败、在线替换、停用与重启恢复。共同事务测试同时执行文件提供者和独立 PostgreSQL 连接，覆盖并发 CAS、锁后撤权、取消回滚、租户隔离、不可变作者、历史 schema 及按代数观察。现有增强、协议、演进和生命周期回归继续验证正式默认组合。
