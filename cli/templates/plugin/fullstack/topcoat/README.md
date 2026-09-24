# __TITLE__

这是 AIO 的 Topcoat Rust `process` 全栈插件模板。模板固定 Topcoat 0.6.2：AIO 当前固定的 `nightly-2026-05-25` 尚未包含 Topcoat 0.8.1 所需的 `int_format_into` 标准库 feature；升级 AIO Rust 工具链后再同步升级 Topcoat。

```bash
cargo generate-lockfile
sh scripts/build.sh
aio plugin validate .
```

后端在 `src/main.rs` 中提供 `/health`、`/aio/describe`、`/api/counter` 与 `/api/context`；`frontend/` 通过 AIO Web SDK 调用这些接口。构建产物为 Linux x86_64 ELF `dist/server` 和 `dist/frontend/`。

Topcoat 默认读取 `PORT`，模板启动时会把 AIO 注入的 `AIO_PLUGIN_PORT` 映射过去。正式发布时使用 `aio plugin package .` 生成 `.aio-plugin` 包。
