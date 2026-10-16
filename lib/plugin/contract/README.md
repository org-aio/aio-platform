# 运行时插件契约

语言无关的 `aio:plugin@2.0.0` WIT。浏览器资源和服务端 Component 是同包中的不同产物。此 crate 不包含宿主、数据库驱动或 UI 框架；身份、租户、权限和业务实现均由插件提供。

WIT 的接口名是跨语言协议符号，来源 UUID 是安装身份，二者都不替代 Rust 进程内 Dill 的 TypeId。

原生客户端可以启用 `grpc` feature 使用 `grpc::enhancements_client::EnhancementsClient`。接口由 `proto/enhancements.proto` 和构建时的 Protobuf 工具生成，支持列举路径与执行增强，返回实际执行版本。其他语言直接消费该 `.proto`。未启用此 feature 时不会引入 Tonic 或 Protobuf 工具，Wasm 契约保持轻量。
