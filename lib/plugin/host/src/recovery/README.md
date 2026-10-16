# 启动配置恢复

该模块是宿主的启动机制，不注册第二个容器，也不依赖 PostgreSQL、业务身份或脚本执行器。产品负责解析其配置、选择具体插件类型和决定何时退出。先看 `ConfigurationJournal`，再看 `storage.rs` 的本地文件边界。

```rust,ignore
let mut journal = ConfigurationJournal::open(&recovery_directory, &source_path)?;
let bytes = read_configuration(&source_path)?;
let digest = configuration_digest(&bytes);
if journal.blocked(&digest) {
    // 使用 confirmed 中的配置，按当前产品契约重新校验；不能默认扩大能力。
}
let attempt = journal.begin(digest)?;
// 解析配置，完成资源、路由及监听准备；失败时 reject，随后由新进程恢复。
journal.confirm(&attempt, validated_configuration)?;
```

`begin` 在执行候选前落盘，未完成或已拒绝的内容不会自动重试。`confirm` 必须在运行时提交成功之后调用，不能只凭配置解析成功就确认。提交失败后保留旧 confirmed；当前进程不应在无法证明资源清理成功时再次冷启动另一套宿主。写入失败会令 journal 不健康，调用方应停止服务并由新进程重新读取持久状态。

记录通过临时文件、文件同步和原子替换提交；Unix 还同步父目录。跨平台掉电耐久性仍以操作系统及文件系统保证为准。使用操作系统文件锁，不以 PID 文件判断存活，也不能通过删除 `owner.lock` 抢占。宿主必须持有 journal 到停止资源结束；恢复目录属于部署管理员，不得交给租户或插件写入。

`inspect` 是不创建文件的只读快照。停服后，运维入口可显式 `allow_retry`，或先验证 `confirmed.value` 再调用 `restore` 恢复部署文件。命令和正常宿主必须使用相同的目录与来源路径；不同来源、损坏记录和未知记录版本均拒绝，不回退为全量默认配置。恢复配置不撤销数据库迁移、外部副作用，也不更换宿主二进制。

验证：`cargo test -p az-plugin-host --features server recovery::tests`，覆盖进程间独占、未完成记录、拒绝与恢复、过期令牌、写入故障、来源隔离和有界读取。
