# 脚本增强执行器

`az-plugin-enhancement` 执行 Rhai 和 TypeScript 宿主能力。它不依赖 HTTP、数据库、UI 或业务插件，宿主负责版本存储、授权和激活。

先看 `src/model.rs` 中的版本包，再看 `src/execution.rs` 的执行器契约、`ScriptRuntime::prepare` 与 `src/invocation.rs` 的调用链。每个路径必须包含协议版本，例如 `/v1/text/normalize`。Rhai 导出 `fn invoke(input, context)`，TypeScript 导出 `export function invoke(input, context)`；输入输出均为 JSON。TypeScript 使用 Oxc 转译及 QuickJS 执行，不需要 Node 或外部 npm 包。

`ScriptExecutor` 按输入语言编译为 `ScriptProgram`，包验证和执行分发不依赖具体解释器。执行器用 Dill 和具体类型的 TypeId 注册；重复类型和语言匹配歧义被拒绝，不存在内置实现回退。独立工具可以使用 `default_runtime()?.prepare(package)`；宿主则在自己的容器中选择 `RhaiExecutor`、`TypeScriptExecutor` 或替代实现，并注册 `ScriptRuntime`。所有持久版本恢复也使用这一个运行时。

默认 feature 包含两种语言。关闭默认 feature 后，通用包契约、样例验证、输入输出配额和执行器接口仍可使用，不会编译 Rhai、Oxc 或 QuickJS。单独启用 `rhai` 或 `typescript` 可只构建对应实现。语言表示源码格式，不是原生插件身份；原生执行器属于可信宿主代码，必须履行独立状态与资源限制约定，不能把同进程的任意原生实现描述为沙箱。

候选包必须通过全部契约样例，普通升级还会重放旧包样例。样例验证不能证明任意输入行为相同，破坏性变化应使用新版本路径。每次调用重新创建脚本作用域，不共享跨租户全局变量。脚本不获得文件、网络、进程、环境变量、模块加载和宿主管理接口。操作数、时间、递归、源码和输入输出均有上限，QuickJS 另有堆与栈限制。

这提供可回滚的运行时算法增强，不代表任意 AI 生成代码已经可信，也不能用脚本替换原生执行器、修改权限或自行突破配额。AI 可以生成并提交候选，是否激活由宿主现有权限边界决定。

## 组合增强

两种语言均可调用 `call_enhancement(path, input)`，同步返回目标增强的 JSON 输出。源码对应的 `Enhancement.calls` 必须声明直接依赖；依赖必须在同一发布包中，重复、缺失、循环和超过 8 层的依赖在候选验证时被拒绝。TypeScript 编辑器可声明 `declare function call_enhancement(path: string, input: unknown): unknown`，这不会导入外部模块。可运行示例见 `examples/composition.json`，其调用链为 Rhai → TypeScript → Rhai。

宿主为每次请求创建 `prepared.invocation(context, grants)`，再调用 `invocation.invoke(path, input)`。`grants` 必须来自真实调用者，包含入口及全部传递依赖；候选中的 `calls` 只声明需要什么，不能授予权限。脚本修改传入的 context 不会改变后续调用的可信身份。整条调用链持有同一个不可变版本快照，升级与回滚在下一次外部请求生效。

调用链共享 100 ms 截止时间、最多 64 次调用（含入口）、最多 8 层和累计 1 MiB JSON 传输额度；每次输入或输出仍最多 64 KiB。每个 JS 上下文仍有独立堆上限，嵌套数量受深度限制。捕获异常不会重置额度。宿主可调用 `invocation.cancel()`，默认执行器通过中断检查协作退出；自定义原生 `ScriptProgram` 必须使用传入的 `ScriptInvocation` 并定期 `check()`，不能自行建立新的版本、权限或预算。此协议不提供原生代码的强制抢占。

当前组合范围是同一包中的运行时增强；文件、数据库和任意远端 MCP/gRPC 调用仍需正式能力提供者及其副作用、幂等和取消契约。

验证：`cargo test -p az-plugin-enhancement`；`cargo test -p az-plugin-enhancement --no-default-features` 验证没有内置引擎时仍可通过注入实现执行、校验候选及拒绝缺失或冲突实现。
