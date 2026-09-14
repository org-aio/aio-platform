# 国内网络与工具链

`aio init` 和 `aio plugin init` 使用随 CLI 内嵌的模板，初始化不联网。默认写入国内依赖源；初始化时传入 `--network global` 使用官方源。配置仅作用于当前项目。

## Kotlin

使用项目内的 `./kotlin`（Windows 为 `kotlin.bat`）。引导脚本依次检查 `AIO_JAVA_HOME`、`JAVA_HOME`、系统 JDK、常见 IDE/SDK 安装目录；自动使用 JDK 25，不会因全局 `JAVA_HOME` 指向 24 就下载 GitHub 上的 JDK。编译工具统一使用 25，模块声明的 `release` 保持各自目标版本。

没有本机 JDK 时，默认从华为云下载固定 OpenJDK 25.0.2，失败回退 OpenJDK 官方源。下载器使用源码锁定的官方 SHA-256，校验成功才解压。工具链版本和摘要位于 `.aio/toolchain/artifacts.tsv`；升级必须同时核验官方发布摘要。该归档是构建工具，不代表最新安全维护版本；可以通过 `AIO_JAVA_HOME` 使用更新的 JDK 25。

全栈模板还会预置 Kotlin CLI 0.12.0-dev-4233 固定使用的 Node 26.5.1 和 pnpm 11.9.0。Node 使用 npmmirror；pnpm 的 GitHub 发布包使用 ghfast.top、gh-proxy.com 下载代理并回退官方源，所有包都校验固定官方摘要。这些下载代理不是官方发布方，不参与信任链。上游 pnpm 11.9.0 未提供 macOS x64 归档，该平台目前可初始化，但全栈前端构建受上游限制。

Maven 源在 `network.module-template.yaml`，npm 源在 `.npmrc`。可按企业网络修改为私有制品仓库。缓存默认位于系统用户缓存目录下，支持复用和并发；失败下载不会作为完成文件生效。

环境变量：

- `AIO_JAVA_HOME`：明确指定本机 JDK 25；不匹配时直接报错。
- `AIO_JDK_DOWNLOAD=1`：使用锁定的 JDK 归档或对应缓存，适合复现工具链和冷启动验证；明确设置的 `AIO_JAVA_HOME` 优先。
- `AIO_TOOLCHAIN_CACHE`：JDK 和已校验归档的缓存目录。
- `KOTLIN_SHARED_CACHE_DIR`：Kotlin 的共享依赖和工具缓存。
- `AIO_DOWNLOAD_ROOT`：企业镜像根地址，按 `artifacts.tsv` 中的文件名提供原始归档；摘要校验始终开启。
- `AIO_NETWORK=global`：仅本次工具下载使用官方源，项目 Maven/npm 配置不改变。
- `AIO_OFFLINE=1`：工具引导只用本机 JDK 和已有归档，缺失时立即报错；依赖编译是否离线由 Kotlin CLI 决定。
- `HTTPS_PROXY` / `HTTP_PROXY`：curl 和 npm 支持的代理；不设置固定代理端口，不自动修改系统代理。Kotlin JVM 内部下载器不保证读取这些环境变量，企业网络应优先配置 Maven 镜像。
- `KOTLIN_CLI_DOWNLOAD_ROOT`：覆盖 Kotlin CLI 自身的 Maven 下载根地址，默认 JetBrains 官方仓库。

下载连接超时为 10 秒，单次总时限 300 秒，最多重试两次，然后尝试下一个源。curl 支持断点续传，失败的 `.part` 文件保留供下次续传，但不会被当成完整归档使用。任何网络都可能限制访问；全断网首次构建需要事先准备工具和依赖缓存。

## TypeScript 与 Rust

TypeScript 的 `.npmrc` 默认使用 npmmirror；先安装 Node 和 pnpm，再运行模板构建命令。CLI 的 npm 分发将平台二进制内嵌到平台包中，不需要 GitHub Release 下载脚本；npm 发布完成后可通过 `npm install -g @zjarlin/aio --registry=https://registry.npmmirror.com` 安装。

Rust 的 `.cargo/config.toml` 默认使用 rsproxy sparse 镜像。Rust 工具链可用 `RUSTUP_DIST_SERVER=https://rsproxy.cn`、`RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup` 下载。Git 源依赖仍需访问各自 Git 服务；企业环境请配置可达仓库或 Git 代理，不能用 crates 镜像替代 Git 仓库。
