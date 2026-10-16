# Runtime Preview

`cargo run -p az-plugin-runtime --features wasm --example preview` 启动仅监听回环地址的开发验证宿主。必须显式提供独立测试数据库、已编译 Component、前端资源和初始化迁移路径。

环境变量：`AIO_TEST_DATABASE_URL`、`AIO_TEST_COMPONENT`、`AIO_TEST_MIGRATION`、`AIO_PREVIEW_ASSETS`；`AIO_PREVIEW_PORT` 默认 4187。

每次启动创建新的测试来源与数据库分区。该工具没有登录功能，不允许部署公网；生产迁移仍需完整身份引导、组合持久化和撤销协议。
