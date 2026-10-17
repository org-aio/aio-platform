# Rust 整包组件契约

`az-plugin-guest` 从 `../contract/wit/plugin.wit` 生成客户绑定。独立插件通过固定 Git revision 引用本 crate，使用 `wasm32-wasip2` 编译；不复制另一份 WIT，不引入 Wasmtime、数据库或宿主实现。

入口为 `Guest`：实现 `describe`、`health`、`lifecycle` 和 `handle` 后，调用 `az_plugin_guest::export!(Component)` 导出整包接口。宿主能力在 `aio::plugin` 下，受包声明及调用上下文授权。

可运行范例与真实宿主验收见 `../runtime/tests/fixture`。修改共享 WIT 后，该夹具和独立插件都重新生成相同绑定。
