# 生命周期测试产物

这是 WIT 生成的测试 Component，不是业务插件。健康检查失败使用 `unhealthy` 构建特征；请求和 prepare 的等待由测试宿主注入，不污染实际插件业务代码。

```sh
cargo build --manifest-path lib/plugin/runtime/tests/fixture/Cargo.toml --release --target wasm32-wasip2 --target-dir target/fixture/healthy
cargo build --manifest-path lib/plugin/runtime/tests/fixture/Cargo.toml --release --target wasm32-wasip2 --features unhealthy --target-dir target/fixture/unhealthy
```

`AIO_TEST_HEALTHY_COMPONENT`、`AIO_TEST_UNHEALTHY_COMPONENT` 指向对应 `wasm32-wasip2/release/release_fixture.wasm`，运行 `cargo test -p az-plugin-runtime --features wasm --test release -- --ignored`。
