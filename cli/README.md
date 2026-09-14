# AIO CLI

`aio` 初始化 Web、Desktop、Server 共用的应用壳，并从一个 Git 仓库发现、安装、更新和卸载前后端插件能力。

插件开发规约见仓库 `docs/plugin/`，可运行示例见 [Dioxus 全栈示例](https://github.com/zjarlin/aio-plugin-dioxus-fullstack) 和 [KMP 全栈示例](https://github.com/zjarlin/aio-plugin-kmp-example)。

```bash
npm install --global @zjarlin/aio
aio init my-app --title "我的应用"
aio plugin init my-plugin --title "业务插件"
aio plugin init my-kmp-plugin --title "KMP 服务" --language kotlin
aio plugin init my-component --title "TS 页面" --language typescript
aio plugin init my-node-plugin --title "Node 服务" --language typescript --runtime process
cd my-app
aio plugin install https://example.com/team/my-plugin.git
aio plugin list
aio plugin validate ../my-plugin
aio plugin publish ../my-component
aio plugin uninstall https://example.com/team/my-plugin.git
```

需要 Node.js 18 或以上版本。npm 包自动安装当前系统的原生 CLI，发布配置见 [npm 分发](../npm/README.md)。从源码安装可在仓库根目录运行 `cargo +nightly install --path cli --locked`。

语言决定默认初始化目标：Rust 固定为源码装配，Kotlin 默认 `process`，TypeScript 默认 `wasm-component`。`--runtime` 是 Kotlin/TypeScript 选择静态页面或非默认目标时的高级覆盖选项，不是常规必填参数；Rust 不接受该选项。

应用仓库的 `aio.toml` 是插件来源真源。每个来源只需配置 `git` 和可选 `rev`；`aio plugin sync` 会拉取仓库、读取根目录 `aio-plugin.toml`、分别发现 client/server Cargo 包、更新 feature 依赖并生成 Dill 注册入口。`aio plugin validate` 则为 Rust、Kotlin 和 TypeScript 仓库提供相同的无副作用协议校验。

Rust 源码插件由应用编译期装配；页面扩展实现 `ApplicationPlugin` 后由 Dill 聚合，Service 和 Controller 按具体类型注册和构造，唯一性只由 `TypeId` 决定。Kotlin 与 TypeScript 模板生成可在线替换的 `page-definition`、`wasm-component` 或 `process` 仓库，并默认写入市场元数据。`aio plugin publish` 从环境读取宿主地址和来源绑定凭证，验证清单与 artifact 后确认当前字节属于完整 Git SHA，再 gzip 上传并轮询到激活。安装器不执行远端仓库脚本；Git 地址、Cargo 包名和页面 id 分别只用于来源、构建和业务导航。
