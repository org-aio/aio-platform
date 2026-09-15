# AIO CLI

`aio` 初始化 Web、Desktop、Server 共用的应用壳，并从一个 Git 仓库发现、安装、更新和卸载前后端插件能力。

插件开发规约见仓库 `docs/plugin/`，可运行示例见 [Dioxus 全栈示例](https://github.com/zjarlin/aio-plugin-dioxus-fullstack) 和 [KMP 全栈示例](https://github.com/zjarlin/aio-plugin-kmp-example)。

```bash
npm install --global @zjarlin/aio --registry=https://registry.npmmirror.com
aio init my-app --title "我的应用"
aio plugin init my-plugin --title "业务插件"
aio plugin init my-kmp-plugin --title "KMP 服务" --language kotlin
aio plugin init my-component --title "TS 页面" --language typescript
aio plugin init my-nuxt-plugin --framework nuxt
aio plugin init my-next-plugin --framework next
aio plugin init my-node-plugin --title "Node 服务" --language typescript --runtime process
cd my-kmp-plugin
aio plugin dev . --debug
cd ../my-app
aio plugin install https://example.com/team/my-plugin.git
aio plugin list
aio plugin validate ../my-plugin
aio plugin publish ../my-component
aio plugin uninstall https://example.com/team/my-plugin.git
```

需要 Node.js 18 或以上版本。npm 包自动安装当前系统的原生 CLI，发布配置见 [npm 分发](../npm/README.md)。完整的 macOS arm64 / Linux x64 分发还包含同版本开发宿主和 Web 资源；仅从源码安装 CLI 不会自动带入 Web 资源，配套构建步骤见 [开发沙箱](../docs/development/README.md)。

三种语言默认生成全栈插件。`--framework nuxt|next` 选择框架原生全栈示例，自动选择 TypeScript；不能与其他语言、`--runtime` 或非全栈 `--kind` 组合。Nuxt 使用 Nitro 服务端路由，Next.js 使用 App Router Route Handlers。两者包含独立开发、共享模型、真实计数接口、构建、类型检查和接口测试。AIO 内嵌入口是静态页面，通过宿主通信桥调用框架后端，不代理 SSR、Server Actions 或框架客户端路由；Next 模板需要支持 `temporary_storage_mb` 的新版宿主。

Rust 源码插件使用 `--kind system`；Kotlin/TypeScript 可用 `--runtime` 选择静态页面、Component 或进程服务。

初始化完全离线，默认 `--network china`，生成项目级国内依赖源；`--network global` 使用官方源。Kotlin 模板自动复用本机 JDK 25、校验下载工具归档并预置前端工具，详情见生成项目的 `NETWORK.md` 或 [网络与工具链](templates/toolchain/README.md)。不修改全局 JAVA_HOME、npm、Cargo 或代理设置。

应用仓库的 `aio.toml` 是插件来源真源。每个来源只需配置 `git` 和可选 `rev`；`aio plugin sync` 会拉取仓库、读取根目录 `aio-plugin.toml`、分别发现 client/server Cargo 包、更新 feature 依赖并生成 Dill 注册入口。`aio plugin validate` 则为 Rust、Kotlin 和 TypeScript 仓库提供相同的无副作用协议校验。

Rust 源码插件由应用编译期装配；页面扩展实现 `ApplicationPlugin` 后由 Dill 聚合，Service 和 Controller 按具体类型注册和构造，唯一性只由 `TypeId` 决定。Kotlin 与 TypeScript 模板生成可在线替换的 `page-definition`、`wasm-component` 或 `process` 仓库，并默认写入市场元数据。`aio plugin publish` 从环境读取宿主地址和来源绑定凭证，验证清单与 artifact 后确认当前字节属于完整 Git SHA，再 gzip 上传并轮询到激活。安装器不执行远端仓库脚本；Git 地址、Cargo 包名和页面 id 分别只用于来源、构建和业务导航。

## 插件市场中的 CLI

运行 `aio helper install` 注册 `aio://install/<id>?version=<版本>`；网页点击后在本机终端确认安装。`aio tool install <id> --version <版本>`、`aio tool list`、`aio tool uninstall <id>` 使用共享安装库。完整发布、依赖检查和恢复说明见 [本机 CLI 市场](../docs/tools/README.md)。
