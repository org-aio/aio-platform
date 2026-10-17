# 整包首次引导

`ComponentBootstrapPlugin` 让部署方在没有市场界面时装载本地 v2 整包。它是可省略的原生装配插件，依赖 `ComponentsPlugin` 和组件存储，使用同一个 Dill 容器、执行器、发布验证、安装事务与失败补偿。

应用组合层追加：

```rust,ignore
runtime_plugins.push(Arc::new(ComponentBootstrapPlugin(vec![BootstrapComponent {
    path: "bundles/dictionary.aio-plugin".into(),
    digest: expected_digest,
    tenants: vec!["default".into()],
}])));
```

`digest` 是打包器输出的规范整包 SHA256，不是压缩文件 SHA256。`tenants` 必须明确列举，不默认面向所有租户；配置属于部署方信任边界，不能交给普通插件或脚本自行写入。高权限 `management`、`identity_provider` 声明仍被现有发布准入拒绝。

## 执行与恢复

1. 启动资源前读取全部文件，核对摘要、清单、宿主版本、权限、重复来源/租户和父子循环。只接受普通文件，单文件最多 96 MiB、单包产物最多 64 MiB，整批元数据、清单与 Base64 产物文本最多 256 MiB；最多 64 包，每包 128 租户。
2. 按父子顺序检查执行器并验证候选。首次发布只补充版本及缺失的发布指针，不覆盖市场已有的最新版本。整批可运行候选发布成功后才开始租户安装；发布记录可以先于安装持久化。
3. 每项成功安装与 `component_initializations(tenant_id, git)` 在同一事务提交。记录使用 Git 业务身份，不保存原生 `TypeId`，也不依附于可删除的来源 UUID。
4. 只给从未安装的声明租户执行首次安装。手动安装同样记入历史；升级、回滚、停用、卸载以及来源删除后，引导均不重置租户选择。移除引导配置不会卸载已有实例。
5. 缺少执行器或父包尚未启用时暂缓，宿主继续启动。重新装配执行器会重建依赖并重试未完成项；父包启用后可重启宿主或重新装配引导插件。v2 父包还必须已经恢复为活动实例。歧义执行器、损坏包、真实验证/激活/提交失败仍返回错误。

这不是跨包事务：前面的安装提交后，后面的安装失败不会回滚已提交版本或外部数据库迁移。下次成功启动从未完成项继续；单项提交失败不会写入首次安装记录，沿用执行器补偿。产品的运行时组合恢复记录可能要求先执行显式 `runtime retry`，不能绕过其启动恢复约束。

升级时从现存 v2 安装记录回填历史，保留停用与回滚状态；旧版本已经卸载且没有历史的安装无法追溯，部署方不应把这些来源重新加入首次引导。声明存在期间文件必须保留且通过校验，即使该租户已经完成安装。

## 产品配置

`aio-idea` 在 `AIO_CONFIG` 指定的 `aio.toml` 读取：

```toml
[[bootstrap_components]]
path = "bundles/dictionary.aio-plugin"
digest = "<打包器输出的 64 位小写摘要>"
tenants = ["default"]
```

相对路径基于配置文件目录。需配置 `AIO_COMPONENT_DATABASE_URL`；没有声明时不装配引导插件。`aio.toml` 的整包声明在进程启动时读取；执行器开关仍由原有 `AIO_RUNTIME_COMPOSITION` 在线装配流程管理。

## 验证边界

`lib/plugin/host/tests/component_execution/bootstrap.rs` 使用替代执行器、真实 HTTP/PostgreSQL，覆盖旧引导包与最新发布共存、租户回滚/停用/卸载、历史回填、来源删除、在线执行器恢复、延迟提交失败后的续装、父子排序与整批输入拒绝。它随 `component_executors` 进入已有 CI。

这个入口解决首次装载，不代表市场、身份或其余系统 UI 已经迁为动态整包，也不提供原生宿主二进制自更新。
