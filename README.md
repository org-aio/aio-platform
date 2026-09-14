# aio-platform

AIO 的可复用运行时、协议、SDK 和开发工具。公网产品入口归 [aio-idea](https://github.com/zjarlin/aio-idea)，业务应用归各自的 `aio-plugin-*` 仓库。

## 边界

- `lib/plugin/contract`：`aio:plugin@2.0.0` WIT 和宿主调用上下文。
- `lib/plugin/runtime`：Wasmtime、能力授权、事务与对象存储代理；不依赖业务插件实现。
- `lib/plugin/core`：Rust 进程内的 Dill/TypeId 扩展基础库，不是在线业务插件。
- `lib/plugin/manifest`、`package`、`bundle`：现役两种包的清单、编码与验证。
- `lib/plugin/host`：通用服务端宿主与浏览器壳；产品通过中立身份接口和显式配置装配。
- `lib/plugin/development`、`cli`、`host`：开发模型、依赖解析、增量调度和配套开发宿主。
- `delivery`：独立生产构建服务，负责工具链和发布任务执行。
- `sdk/web`：隔离 iframe 的二进制通信 SDK。
- `lib/dioxus-admin-workbench`：独立仓库的壳布局与基础组件 crates，不属于业务插件。

Studio 的编辑器、数据库迁移、生成业务和人工实现已从平台移到独立 `aio-plugin-studio`，Git 历史保留。平台不再包含 `app/`、`lib/biz/` 或 `generated/apps/`。

## 全栈插件

一个功能仓库包含 `frontend/`、`backend/`、`shared/`，作为同一个版本安装和回滚。前端交付完整 HTML/JS/Wasm 资源，由沙箱 iframe 执行；后端默认使用 Wasm Component，原生 SDK/长任务可选择受控 process。

`PageDefinition` 只描述页面入口、场景根、菜单路径、权限和挂载面。真正的组件、状态和交互在插件自身实现，不再将完整应用限制为标题、文本和按钮 JSON。KMP 全栈示例使用真实 `@Composable`、`ComposeViewport` 和 Ktor/JVM 后端；Component 插件通过 WIT 接口访问宿主能力。

Cargo 依赖决定进程内链接；清单描述产物与能力申请；租户组合决定运行时激活。三者不是同一种插件机制。

## 当前验证

```sh
cargo test --workspace
node scripts/check-boundaries.mjs
```

Kotlin Component 与 PostgreSQL 的集成测试见 `lib/plugin/runtime/tests/README.md`；真实前端预览见 `lib/plugin/runtime/examples/README.md`。

产品保留静态系统插件组合；通用包生命周期、浏览器壳、发布队列和开发工具归平台。现役生产包继续可用，本次抽取不要求另建 ABI。历史整改记录见 [整改记录](docs/refactor/README.md)，当前本地运行入口见 [开发沙箱](docs/development/README.md)。

插件开发入口：`.agents/skills/aio-plugin-development/SKILL.md`。命令名保持 `aio`。
