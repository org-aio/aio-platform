# aio-platform / AIO Platform

AIO 的可复用运行时、协议、SDK 和开发工具。公网产品入口归 [aio-idea](https://github.com/zjarlin/aio-idea)，业务应用归各自的 `aio-plugin-*` 仓库。

AIO's reusable runtime, protocols, SDKs and development tools. The public product entry belongs to [aio-idea](https://github.com/zjarlin/aio-idea); business applications belong to their respective `aio-plugin-*` repositories.

## 边界 / Boundaries

- `lib/plugin/contract`：`aio:plugin@2.0.0` WIT 和宿主调用上下文。
- `lib/plugin/runtime`：Wasmtime、能力授权、事务与对象存储代理；不依赖业务插件实现。
- `lib/plugin/core`：Rust 进程内的 Dill/TypeId 扩展基础库，不是在线业务插件。
- `lib/plugin/manifest`、`package`、`bundle`：现役两种包的清单、编码与验证。
- `lib/plugin/host`：通用服务端宿主与浏览器壳；产品通过中立身份接口和显式配置装配。
- `lib/plugin/development`、`cli`、`host`：开发模型、依赖解析、增量调度和配套开发宿主。
- `delivery`：独立生产构建服务，负责工具链和发布任务执行。
- `sdk/web`：隔离 iframe 的二进制通信 SDK。
- `lib/dioxus-admin-workbench`：独立仓库的壳布局与基础组件 crates，不属于业务插件。

- `lib/plugin/contract`: the `aio:plugin@2.0.0` WIT and host invocation context.
- `lib/plugin/runtime`: Wasmtime, capability authorization, transaction and object-store proxies; independent of business plugin implementations.
- `lib/plugin/core`: an in-process Rust Dill/TypeId extension base library, not an online business plugin.
- `lib/plugin/manifest`, `package`, `bundle`: manifests, encoding and verification for the two current package formats.
- `lib/plugin/host`: the generic server-side host and browser shell; products are assembled through a neutral identity interface and explicit configuration.
- `lib/plugin/development`, `cli`, `host`: the development model, dependency resolution, incremental scheduling and companion development hosts.
- `delivery`: the standalone production build service that executes toolchain and release tasks.
- `sdk/web`: the binary communication SDK for sandboxed iframes.
- `lib/dioxus-admin-workbench`: shell layout and base component crates from a separate repository, not a business plugin.

Studio 的编辑器、数据库迁移、生成业务和人工实现已从平台移到独立 `aio-plugin-studio`，Git 历史保留。平台不再包含 `app/`、`lib/biz/` 或 `generated/apps/`。

Studio's editor, database migrations, generated business and human-owned implementations have moved out of the platform into the standalone `aio-plugin-studio`, with Git history preserved. The platform no longer contains `app/`, `lib/biz/` or `generated/apps/`.

## 全栈插件 / Full-Stack Plugins

一个功能仓库包含 `frontend/`、`backend/`、`shared/`，作为同一个版本安装和回滚。前端交付完整 HTML/JS/Wasm 资源，由沙箱 iframe 执行；后端默认使用 Wasm Component，原生 SDK/长任务可选择受控 process。

A feature repository contains `frontend/`, `backend/` and `shared/`, installed and rolled back as a single version. The frontend ships complete HTML/JS/Wasm assets executed in a sandboxed iframe; the backend uses a Wasm Component by default, with a controlled process available for native SDKs/long-running tasks.

`PageDefinition` 只描述页面入口、场景根、菜单路径、权限和挂载面。真正的组件、状态和交互在插件自身实现，不再将完整应用限制为标题、文本和按钮 JSON。KMP 全栈示例使用真实 `@Composable`、`ComposeViewport` 和 Ktor/JVM 后端；Component 插件通过 WIT 接口访问宿主能力。

`PageDefinition` only describes the page entry, scenario root, menu path, permissions and mounting surface. Real components, state and interactions live in the plugin itself — a full application is no longer reduced to title/text/button JSON. The KMP full-stack example uses real `@Composable`, `ComposeViewport` and a Ktor/JVM backend; Component plugins access host capabilities through the WIT interface.

Cargo 依赖决定进程内链接；清单描述产物与能力申请；租户组合决定运行时激活。三者不是同一种插件机制。

Cargo dependencies decide in-process linking; the manifest describes artifacts and capability requests; tenant composition decides runtime activation. These are three different plugin mechanisms.

## 当前验证 / Current Verification

```sh
cargo test --workspace
node scripts/check-boundaries.mjs
```

Kotlin Component 与 PostgreSQL 的集成测试见 `lib/plugin/runtime/tests/README.md`；真实前端预览见 `lib/plugin/runtime/examples/README.md`。

Kotlin Component and PostgreSQL integration tests live in `lib/plugin/runtime/tests/README.md`; real frontend previews in `lib/plugin/runtime/examples/README.md`.

产品保留静态系统插件组合；通用包生命周期、浏览器壳、发布队列和开发工具归平台。现役生产包继续可用，本次抽取不要求另建 ABI。历史整改记录见 [整改记录](docs/refactor/README.md)，当前本地运行入口见 [开发沙箱](docs/development/README.md)。

The product keeps its static system plugin composition; the generic package lifecycle, browser shell, release queue and development tools belong to the platform. Current production packages keep working — this extraction does not require a new ABI. Historical remediation notes are in [整改记录](docs/refactor/README.md); the current local run entry is in [开发沙箱](docs/development/README.md).

插件开发入口：`.agents/skills/aio-plugin-development/SKILL.md`。命令名保持 `aio`。

Plugin development entry point: `.agents/skills/aio-plugin-development/SKILL.md`. The command name stays `aio`.
