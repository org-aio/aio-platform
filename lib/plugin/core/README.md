# Plugin Core

Cargo 工件：`az-plugin-core`

本 crate 提供 Bevy 风格的 `App<T>`、`Plugin<T>`、`PluginGroup<T>`，以及统一 HTTP 边界、共享 PostgreSQL 句柄、内置 Toasty 模型、动态模型和 JSONB 记录。

`Plugin<T>` 继承 `Any`。Dill 使用 `AllOf<dyn Plugin<T>>` 聚合实现，`PluginGroupBuilder` 只保存 `TypeId` 以恢复显式顺序，`App` 也只按 `TypeId` 拒绝重复插件。`Plugin::comment()` 可以提供诊断描述，但不参与注册、查找或持久化。

`Plugin::dependencies()` 声明必须先构建的具体插件类型。`App` 在执行构建前拒绝缺失依赖与循环，并保持无依赖关系插件的原有装配顺序。服务实现之间的依赖继续交给 Dill；不能用具体插件依赖阻止同一服务接口的合法替换。构建阶段尚不提供任意状态的事务回滚。

原生宿主可使用 `az-plugin-core = { ..., default-features = false }` 只引入插件装配和 Dill。数据库、记录与 HTTP 模块由默认启用的 `infrastructure` feature 提供。

```bash
cargo test -p az-plugin-core
cargo test -p az-plugin-core --no-default-features
```
