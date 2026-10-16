# 脚本增强执行器

`az-plugin-enhancement` 执行 Rhai 和 TypeScript 宿主能力。它不依赖 HTTP、数据库、UI 或业务插件，宿主负责版本存储、授权和激活。

先看 `src/model.rs` 中的版本包，再看 `src/execution.rs` 的执行器契约、`ScriptRuntime::prepare` 与 `PreparedPackage::invoke`。每个路径必须包含协议版本，例如 `/v1/text/normalize`。Rhai 导出 `fn invoke(input, context)`，TypeScript 导出 `export function invoke(input, context)`；输入输出均为 JSON。TypeScript 使用 Oxc 转译及 QuickJS 执行，不需要 Node 或外部 npm 包。

`ScriptExecutor` 按输入语言编译为 `ScriptProgram`，包验证和执行分发不依赖具体解释器。执行器用 Dill 和具体类型的 TypeId 注册；重复类型和语言匹配歧义被拒绝，不存在内置实现回退。独立工具可以使用 `default_runtime()?.prepare(package)`；宿主则在自己的容器中选择 `RhaiExecutor`、`TypeScriptExecutor` 或替代实现，并注册 `ScriptRuntime`。所有持久版本恢复也使用这一个运行时。

默认 feature 包含两种语言。关闭默认 feature 后，通用包契约、样例验证、输入输出配额和执行器接口仍可使用，不会编译 Rhai、Oxc 或 QuickJS。单独启用 `rhai` 或 `typescript` 可只构建对应实现。语言表示源码格式，不是原生插件身份；原生执行器属于可信宿主代码，必须履行独立状态与资源限制约定，不能把同进程的任意原生实现描述为沙箱。

候选包必须通过全部契约样例，普通升级还会重放旧包样例。样例验证不能证明任意输入行为相同，破坏性变化应使用新版本路径。每次调用重新创建脚本作用域，不共享跨租户全局变量。脚本不获得文件、网络、进程、环境变量、模块加载和宿主管理接口。操作数、时间、递归、源码和输入输出均有上限，QuickJS 另有堆与栈限制。

这提供可回滚的运行时算法增强，不代表任意 AI 生成代码已经可信，也不能用脚本替换原生执行器、修改权限或自行突破配额。AI 可以生成并提交候选，是否激活由宿主现有权限边界决定。

验证：`cargo test -p az-plugin-enhancement`；`cargo test -p az-plugin-enhancement --no-default-features` 验证没有内置引擎时仍可通过注入实现执行、校验候选及拒绝缺失或冲突实现。
