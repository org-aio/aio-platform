# 项目初始化

负责生成可直接构建的 Web/Desktop/Server 应用仓库，以及 Rust、Kotlin Toolchain、TypeScript 插件仓库。模板消费正式插件协议与独立 workbench 壳，不复制宿主组件、CSS 或私有模型。

三种语言默认生成全栈插件：Rust 为 Dioxus + Component，Kotlin 为 Compose Web + Ktor，TypeScript 为浏览器前端 + Node 服务，分别拆分前端、后端和共享模型。生成的 `aio-delivery.toml` 是自动发现的显式标记；推送默认分支后由独立构建服务发布，无需 GitHub Actions。Rust 系统源码插件使用 `--kind system`，页面扩展实现 trait 后由 Dill 按具体 `TypeId` 聚合。

`--runtime` 保留 Kotlin 与 TypeScript 的 `page-definition`、`wasm-component`、`process` 高级模板。Kotlin Component 当前明确标记为预览。模板生成锁定工具链、功能目录 README、测试构建入口和本地构建说明；生产安装器只接受通过验证的二进制包。

`network/` 在模板生成后统一写入项目级网络配置，默认 `china`，可用 `--network global` 切换。所有初始化路径完全离线。Kotlin 各模板共用一份包装器、工具锁文件与 JDK 引导实现；下载校验、缓存、续传和平台适配集中在 `templates/toolchain/`，不散落到业务构建步骤。

`--kind cli` 默认 TypeScript + Node.js，生成独立 CLI、锁文件和 npm/AIO 市场工作流；`--adopt` 为现有 npm CLI 增加交付配置。CLI 的公开 npm 发布使用 GitHub 托管工作流与 OIDC，首次通过 `aio tool release setup` 绑定包所有权。CLI 不使用普通宿主插件的 aio-delivery 构建路径。
