# Plugin Manifest

Cargo 工件：`az-plugin-manifest`

页面入站统一使用 `parse_page_definitions`：支持单个场景树、场景树数组和协议页面列表，校验后返回可持久化的 `Vec<PageDefinition>`。树的 `children` 只用于编写导航，目录路径由公共层展开；叶子权限不会丢失或从目录隐式继承。模型、限制及实现见 [导航模块](src/navigation/README.md)。

本 crate 提供语言无关的 `aio-plugin.toml`、`PageDefinition`、`PluginRequest`、`ComponentResponse` 和运行目标模型。`[plugin.marketplace]` 是在线发布时写入 PostgreSQL 的市场展示元数据，和运行时、能力及子插件声明处于同一份可审计清单。`validation` feature 提供仓库 artifact 和子插件依赖图校验，以及不依赖具体引擎的 `WasmArtifactValidator` 契约。`wasm-validation` 额外提供 `WasmtimeValidator`，由 CLI 或内置执行器显式选择；`schema` feature 从同一 Rust 模型生成多语言工具可消费的 JSON Schema。

调用 `validate_repository(root, Some(&validator))` 时，Wasm 验证器负责 ABI、执行限额及实际页面读取；仓库层统一复核非空页面、页面模型、清单声明与前端文件。传入 `None` 可以检查静态或进程仓库，遇到 Wasm 产物时会失败，即使当前编译启用了内置 Wasmtime 也不会隐式选用。CLI 使用 `Some(&az_plugin_manifest::WasmtimeValidator)`；宿主使用当前 Dill 容器中的 `WasmExecutor`，因此替代执行器无需打开内置引擎 feature。
