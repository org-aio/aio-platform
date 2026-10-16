# Runtime Integration Tests

`component` 使用真实 Kotlin Component 和独立 PostgreSQL 测试库。它默认忽略，不将缺少环境当作通过。

设置 `AIO_TEST_DATABASE_URL`、`AIO_TEST_COMPONENT`、`AIO_TEST_MIGRATION` 后运行：

```sh
cargo test -p az-plugin-runtime --features wasm --test component -- --ignored --nocapture
cargo test -p az-plugin-runtime --features wasm --lib -- --include-ignored
```

测试库需要预先撤销 public schema 的 PUBLIC 权限。测试只创建随机命名的插件角色和 schema，不连接生产库。

`release` 使用 `fixture/` 构建的真实 WIT Component，覆盖健康检查失败、坏包、缺少页面、取消准备、旧请求排空、二进制响应、环境隔离、停用与旧页面摘要失效；CI 显式运行，不依赖数据库。

`registry` 在独立 PostgreSQL 中验证包与激活版本持久化、重启恢复、失败保留活动版本、租户拒绝、停用撤销及跨实例请求排空。将 `AIO_TEST_HEALTHY_COMPONENT`、`AIO_TEST_UNHEALTHY_COMPONENT` 设置为两个 fixture 的绝对路径，再运行 `cargo test -p az-plugin-runtime --features wasm --test registry -- --ignored`。

`bundle_component` 使用真实 Kotlin 整包（Compose 资源、Component、SQL）及 PostgreSQL，覆盖前后端同摘要、能力拒绝、租户隔离、替换和停用后重新激活的数据持久化。额外设置 `AIO_TEST_KMP_ROOT` 为示例仓库、`AIO_TEST_KMP_COMMIT` 为其完整提交 SHA，运行 `cargo test -p az-plugin-runtime --features wasm --test bundle_component -- --ignored`。测试不执行源码构建或生产发布。
