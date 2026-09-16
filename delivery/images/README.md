# 固定工具链镜像

镜像在部署时构建一次，工作进程通过内容摘要使用。基础镜像通过 BASE_IMAGE 显式传入固定摘要，源码构建没有凭证和宿主 socket。

Rust 镜像预装 Dioxus CLI 0.7.9、wasm-bindgen 0.2.128、Binaryen 127 和 esbuild 0.27.3。构建时禁用 Dioxus 自动下载工具，避免隐式工具链变化；新增下载均验证固定摘要。

Kotlin 镜像预装 Kotlin CLI 0.12.0-dev-4233，以及该版本指定的 pnpm 11.9.0 和 Node 26.5.1。启动脚本只在任务缓存挂载后复制工具归档，下载缓存键遵循锁定 CLI 的 URL + `V1` SHA256 前十位规则；升级 CLI 时同步核对规则。CLI 本身使用只读镜像目录，插件依赖仍在各来源独立缓存中解析。镜像构建上下文使用本目录，确保包含 `kotlin-entrypoint.sh`。

`rust-process.Dockerfile` 从固定摘要的 Rust/Fullstack 镜像复制 Node 作者工具、cargo-zigbuild 0.23.4 与 Zig 0.16.0，保留 Rust 镜像的 Dioxus/WebAssembly 工具；生成新的 AIO_BUILD_IMAGE_RUST 摘要。插件产物不包含这些构建工具。
