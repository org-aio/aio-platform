# Plugin Manifest

Cargo 工件：`az-plugin-manifest`

页面入站统一使用 `parse_page_definitions`：支持单个场景树、场景树数组和协议页面列表，校验后返回可持久化的 `Vec<PageDefinition>`。树的 `children` 只用于编写导航，目录路径由公共层展开；叶子权限不会丢失或从目录隐式继承。模型、限制及实现见 [导航模块](src/navigation/README.md)。

本 crate 提供语言无关的 `aio-plugin.toml`、`PageDefinition`、`PluginRequest`、`ComponentResponse` 和运行目标模型。`[plugin.marketplace]` 是在线发布时写入 PostgreSQL 的市场展示元数据，和运行时、能力及子插件声明处于同一份可审计清单。`validation` feature 提供仓库 artifact、子插件依赖图和 Wasm Component ABI 校验；`schema` feature 从同一 Rust 模型生成多语言工具可消费的 JSON Schema。
