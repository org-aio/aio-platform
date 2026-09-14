# 插件本地开发沙箱

插件开发只需要插件仓库、配套 AIO CLI 和所选语言工具链。`aio-idea` 是产品装配，不是开发插件的前置仓库。

## 启动

```sh
npm install -g @zjarlin/aio@2026.9.14
aio plugin init my-plugin --language kotlin
cd my-plugin
aio plugin dev .
```

`--language` 支持 `rust`、`kotlin`、`typescript`。初始化不联网，不要求 Git、远端仓库或提交。开发命令会准备项目数据库、构建前后端、启动平台宿主并打开浏览器。

```sh
aio plugin dev . --debug
aio plugin dev . --with ../dependency-plugin
aio plugin dev . --no-watch
aio plugin dev . --offline --no-open --port 4200
```

默认需要本地 Docker 引擎。macOS 可以使用 Docker Desktop 或 Colima；Linux 可以使用 Docker Engine。默认数据库是 PostgreSQL 17.6，绑定回环地址，使用项目独立容器、卷和随机凭据。

已有独立开发数据库时：

```sh
AIO_DEV_DATABASE_URL='postgres://developer:password@127.0.0.1:5432/my_plugin_dev' aio plugin dev .
```

成功启动后，连接保存在 `.aio/dev/database-connection.json`（权限 0600）；以后可直接运行 `aio plugin dev .`。也支持 `--database-url`。首次连接的数据库必须为空，此后只接受同一项目的所有权记录。开发账号需要创建 schema 和隔离角色的权限。不要将开发连接指向生产数据库。

## 本地状态与构建

所有开发状态位于 `.aio/dev/`，包括日志、工具链快照、增量构建状态、版本快照、调试配置记录、宿主数据和密钥。Ctrl+C 或终止信号停止本次启动的进程与数据库容器，保留开发数据。外部提供的数据库不由 CLI 停止。

`aio-dev.toml` 定义语言无关的输入、命令和输出。命令使用参数数组，不经过 shell 拼接。需要 shell 行为时显式调用脚本。

```toml
version = 1

[plugin]
version = "1.0.0"

[frontend]
inputs = ["frontend", "shared", "package.json", "pnpm-lock.yaml"]
command = ["node", "scripts/dev.mjs", "frontend"]
output = "dist/frontend"

[backend]
inputs = ["backend", "shared", "package.json", "pnpm-lock.yaml"]
command = ["node", "scripts/dev.mjs", "backend"]
output = "dist/server.js"

[run]
command = ["node", "{debug_args}", "{artifact}"]
debug_arguments = ["--inspect=127.0.0.1:{debug_port}"]
health = "/health"
```

可选 `[prepare]` 使用相同任务结构，适合锁定依赖安装。`[plugin].version` 用于没有 Cargo/package.json 版本声明的工作区；其他项目从已有版本声明读取。

前端输入变化只编译前端，保留后端实例；后端输入变化先启动候选服务并完成健康检查，再切换版本。共享模型、清单与工具链输入会使相关任务失效。连续保存合并，过期产物不会激活，失败继续保留上一成功页面。恢复原来的有效源码会自动清除失败状态。

构建成功由本地宿主通知浏览器，无需刷新。壳导航和插件路由由通信桥保留。插件内存状态允许在对应实例重载时重置。修改依赖图或版本约束后重新运行命令，让解析器重新确认完整运行集合。

开发产物使用工作区来源和内容摘要；不伪造 Git SHA，不调用市场发布接口，也不会触发公开仓库发现或生产租户升级。正式发布仍执行测试、完整打包和健康检查。

## 依赖与离线

在 `aio-plugin.toml` 中声明必需依赖：

```toml
[[plugin.dependencies]]
git = "https://github.com/example/dependency-plugin.git"
version = "^1.0"
```

既有父插件关系也参与必需依赖解析。只加载当前插件和递归依赖；`--with` 必须覆盖已声明的来源，不扫描其他工作区。尚未发布的依赖必须提供本地路径。没有远端的本地依赖可使用与声明仓库一致的目录名；存在歧义时设置 origin 即可，无需 push。

默认依赖从发布目录选择已验证的包。配置 `AIO_DEV_CATALOG_URL` 可使用其他 AIO 宿主；需要登录的目录通过 `AIO_DEV_CATALOG_SESSION` 接收已有会话 Cookie。首次下载后，`aio-dev.lock` 记录宿主版本、源码 SHA、版本约束结果和包摘要；之后复用锁定包。删除锁文件会重新解析版本。

`*` 接受所有已成功发布版本，也包含自动发布生成的 `0.0.0-dev.<任务号>+<SHA>`。有界范围遵循 semver 规则；要选择预发布系列，需要在约束中明确包含对应预发布版本。

缓存准备完成后使用 `--offline` 解析依赖，不访问发布目录。构建器仍按照自己的输入与缓存运行；断网开发需要先准备对应语言依赖、工具链和容器镜像。缓存包每次复用都会验证摘要，修改缓存不会被当作新的发布版本。

现役 Component 包与进程包均复用正式验证器。Linux x64 进程包在 macOS 通过 Docker 的 amd64 模拟运行；本地工具要求 Node 22+。传输和业务容器关闭网络，仅通过标准输入输出连接宿主 Unix socket，退出清理临时容器和卷。镜像必须锁定摘要。

## 调试

`--debug` 生成独立命名的 `.run/AIO_*.run.xml`，包含沙箱启动、JavaScript 调试和实际 JVM Attach 端口。手工修改过的配置不会被覆盖。终端输出实际 URL、调试端口和日志目录；端口变化时自动更新 CLI 自己生成的配置。

- Kotlin/Wasm：在浏览器或 IntelliJ 的 JavaScript 调试器打开壳页面，在 iframe 的源码映射中找到 `CounterPage.kt`。映射包含本地源码 URL 与源码内容。使用增量编译后重载。
- Ktor/JVM：使用生成的 `AIO JVM Attach` 连接回环 JDWP 端口，再从插件页面发出请求。默认 `suspend=n`，可用 `--jvm-args` 增加 JVM 参数。断点暂停期间不使用生产请求超时重启服务。
- TypeScript：前后端生成 source map；浏览器调试前端，Node Inspector 调试后端。
- Rust：前后端使用 debug 构建，保留符号、增量缓存和运行时诊断；Wasm Component 源码单步能力取决于调试器，本轮不以所有系统上完成源码单步为承诺。

沙箱默认保留隔离 iframe 和正式通信桥。直接打开插件自己的 HTML 不能替代壳内调试与验收。

## 平台与产品边界

| 仓库/模块 | 职责 |
| --- | --- |
| `aio-platform/lib/plugin/host` | 通用会话上下文、安装与版本切换、资源服务、目录更新、发布队列、租户升级、浏览器壳 |
| `aio-platform/lib/plugin/development` | 开发配置、产物、锁文件和依赖解析模型 |
| `aio-platform/cli`、`host`、`delivery` | 开发调度、配套宿主与 Web 分发、生产构建服务 |
| `dioxus-admin-workbench` | 无头布局、导航、主题及共享 UI 组件 |
| `aio-idea/src/host` | 产品身份适配、发布账号策略、默认组合和生产环境配置 |
| `aio-plugin-*` | 业务前端、后端、共享模型、依赖和构建声明 |

平台宿主通过 `HostConfig` 和 `IdentityProvider` 装配。生产和本地沙箱消费同一实现；产品静态系统页面继续由产品组合装配。本次不要求改写现役插件包或数据库业务数据。

## 从源码验证分发

```sh
cargo +nightly build -p az-aio-cli -p aio-host
dx build --package aio-host --platform web --release --no-default-features --features web --debug-symbols false
AIO_DEV_HOST="$PWD/target/debug/aio-host" \
AIO_DEV_WEB_DIST="$PWD/target/dx/aio-host/release/web/public" \
  target/debug/aio plugin dev ../aio-plugin-kmp-example --debug
```

npm 的 macOS arm64 和 Linux x64 分发包含同版本 `aio`、`aio-host` 和 `web/`。仅 `cargo install --path cli` 安装的是 CLI 可执行文件，需要额外提供配套开发宿主与 Web 资源。其他平台保留原有 CLI 能力，不据此声称已完成本轮沙箱调试验收。
