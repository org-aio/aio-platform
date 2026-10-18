# 固定发布启动器

`az-plugin-launcher` 是宿主二进制自我演进的信任根。它必须在任何宿主代码之前
运行，因此只依赖文件系统、systemd 重启与 HTTP 健康入口，不读业务配置、数据库
或插件目录。

## 目录布局

```text
/opt/aio-idea/launcher/aio-launcher   固定启动器副本（current 之外，不随发布替换）
/opt/aio-idea/releases/<40 位 SHA>/   不可变发布物
/opt/aio-idea/current                 指向当前发布的原子符号链接
/opt/aio-idea/switch.json             切换事务记录
```

发布物包含 `release-manifest.json`，列出宿主二进制、前端入口与 `aio.toml` 的
SHA-256。启动器在切换与启动前逐项复核，任何差异都会阻止发布。

## 三个入口

| 入口 | 位置 | 职责 |
| --- | --- | --- |
| `boot` | systemd `ExecStart` | 等待在途切换收敛、校验当前发布，然后 `exec` 成宿主，PID 与日志保持不变 |
| `activate <SHA>` | 部署侧一次性执行 | 校验候选、原子切换、重启服务、等待健康，失败回滚 |
| `repair` | 运维命令 | 把未确认的切换记录显式回滚到上一发布 |

`activate` 不得在承载服务内部运行；服务内只运行 `boot`。切换事务写
`switch.json`：

- `switching`：候选已切换、等待健康确认，同时记录持有者 PID 与进程起始时间；
- `confirmed`：候选通过健康检查，成为当前发布；
- `rolled_back`：候选失败，已回到上一发布并记录原因。

持有者标记把“部署侧仍在执行 activate”和“那次 activate 已崩溃”区分开。`boot`
只在持有者进程消失且超过宽限期后才接管回滚，避免与在途事务竞争。

## 环境变量

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `AIO_LAUNCHER_ROOT` | 必填 | 发布根目录 |
| `AIO_LAUNCHER_BINARY` | `aio-idea` | 发布目录内的宿主二进制名 |
| `AIO_LAUNCHER_HEALTH_URL` | `http://127.0.0.1:3080/health` | 健康入口 |
| `AIO_LAUNCHER_HEALTH_SECONDS` | `300` | 健康等待上限 |
| `AIO_LAUNCHER_RESTART` | `systemctl restart aio-idea.service` | 重启命令；目标发布目录作为最后一个参数追加 |
| `AIO_LAUNCHER_SETTLE_SECONDS` | `330` | 持有者消失后的宽限期 |
| `AIO_LAUNCHER_SERVICE` | 未设置 | 设置后拒绝 `activate`，防止服务内自杀式重启 |

## 验收

`tests/switch.rs` 用替身覆盖成功切换、健康失败回滚、重启命令失败回滚、摘要篡改
拒绝、崩溃修复、路径逃逸拒绝与在途事务等待。真实进程验收另行覆盖真实二进制、
真实 HTTP 健康、`boot` 的 `exec` 语义、失败回滚与服务内 `activate` 拒绝。

启动器不负责业务数据迁移、数据库事务或插件版本；那些由被它启动的宿主与 v2
整包机制处理。
