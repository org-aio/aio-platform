# 脚本增强执行器

`az-plugin-enhancement` 执行 Rhai 和 TypeScript 宿主能力。它不依赖 HTTP、数据库、UI 或业务插件，宿主负责版本存储、授权和激活。

先看 `src/model.rs` 中的版本包，再看 `src/execution.rs` 的执行器契约、`ScriptRuntime::prepare` 与 `src/invocation.rs` 的调用链。每个路径必须包含协议版本，例如 `/v1/text/normalize`。Rhai 导出 `fn invoke(input, context)`，TypeScript 导出 `export function invoke(input, context)`；输入输出均为 JSON。TypeScript 使用 Oxc 转译及 QuickJS 执行，不需要 Node 或外部 npm 包。

`ScriptExecutor` 按输入语言编译为 `ScriptProgram`，包验证和执行分发不依赖具体解释器。执行器用 Dill 和具体类型的 TypeId 注册；重复类型和语言匹配歧义被拒绝，不存在内置实现回退。独立工具可以使用 `default_runtime()?.prepare(package)`；宿主则在自己的容器中选择 `RhaiExecutor`、`TypeScriptExecutor` 或替代实现，并注册 `ScriptRuntime`。所有持久版本恢复也使用这一个运行时。

默认 feature 包含两种语言。关闭默认 feature 后，通用包契约、样例验证、输入输出配额和执行器接口仍可使用，不会编译 Rhai、Oxc 或 QuickJS。单独启用 `rhai` 或 `typescript` 可只构建对应实现。语言表示源码格式，不是原生插件身份；原生执行器属于可信宿主代码，必须履行独立状态与资源限制约定，不能把同进程的任意原生实现描述为沙箱。

候选包必须通过全部契约样例，普通升级还会重放旧包样例。样例验证不能证明任意输入行为相同，破坏性变化应使用新版本路径。每次调用重新创建脚本作用域，不共享跨租户全局变量。脚本不获得文件、网络、进程、环境变量、模块加载和宿主管理接口。操作数、时间、递归、源码和输入输出均有上限，QuickJS 另有堆与栈限制。

这提供可回滚的运行时算法增强，不代表任意 AI 生成代码已经可信，也不能用脚本替换原生执行器、修改权限或自行突破配额。AI 可以生成并提交候选，是否激活由宿主现有权限边界决定。

## 组合增强

两种语言均可调用 `call_enhancement(path, input)`，同步返回目标增强的 JSON 输出。源码对应的 `Enhancement.calls` 必须声明直接依赖；依赖必须在同一发布包中，重复、缺失、循环和超过 8 层的依赖在候选验证时被拒绝。TypeScript 编辑器可声明 `declare function call_enhancement(path: string, input: unknown): unknown`，这不会导入外部模块。可运行示例见 `examples/composition.json`，其调用链为 Rhai → TypeScript → Rhai。

宿主为每次请求创建 `prepared.invocation(context, grants, capabilities)`，再调用 `invocation.invoke(path, input)`。`grants` 必须来自真实调用者，包含入口及全部传递依赖；候选中的 `calls` 只声明需要什么，不能授予权限。脚本修改传入的 context 不会改变后续调用的可信身份。整条调用链持有同一个不可变版本快照，升级与回滚在下一次外部请求生效。

调用链共享 100 ms 脚本计算时限、30 秒总等待期限、最多 64 次调用（含入口和宿主能力）、最多 8 层和累计 1 MiB JSON 传输额度；每次输入或输出仍最多 64 KiB。每个 JS 上下文仍有独立堆上限，嵌套数量受深度限制。捕获异常不会重置额度。宿主可调用 `invocation.cancel()`，默认执行器通过中断检查协作退出；自定义原生 `ScriptProgram` 必须使用传入的 `ScriptInvocation` 并定期 `check()`，不能自行建立新的版本、权限或预算。此协议不提供原生代码的强制抢占。

## 调用宿主能力

两种语言均提供 `call_capability(path, input)`。`Enhancement.capabilities` 声明直接依赖；调用者还需独立授予全部传递依赖。`InvocationCapabilities` 由宿主绑定能力入口和授权快照，默认值拒绝全部。调用前一次性检查整个依赖闭包，未授权或未装配的能力不会执行任何脚本。原生提供者接收宿主固定的请求归属，脚本修改 context 或在 input 中附带用户字段不会改变归属。

每个 `Example.capabilities` 是有序的 `{path,input,output}` 调用记录。候选验证与旧契约重放只使用这些记录；路径、输入、顺序或次数不符均失败。验证永远不连接真实设备、数据库或远端服务，不能把验证候选当作执行副作用的入口。样例只证明给定调用记录下的脚本行为；提供者仍需要真实集成验收。改变已有样例的外部交互契约时应新增版本路径。

等待宿主异步 I/O 暂停脚本计算计时，但仍消耗同一条链的总等待期限、调用次数和 JSON 字节预算。取消会通知宿主适配器并丢弃等待的 Future，解释器退出前仍持有并发配额。已经提交的外部操作不会被假装回滚：写能力必须定义业务幂等标识，设备任务使用固定任务 UUID 重试，取消任务通过独立能力执行。

可运行示例见 `examples/worker-list.json`。平台宿主通过 `CapabilityProvider` 与 Dill 接入原生服务，默认 `WorkerCapabilitiesPlugin` 提供设备列表和任务提交、查询、取消；它不赋予直接文件、SQL、网络或进程访问。任意外部 MCP/gRPC 服务导入、流式任务与通用副作用协调仍待实现。

验证：`cargo test -p az-plugin-enhancement`；`cargo test -p az-plugin-enhancement --no-default-features` 验证没有内置引擎时仍可通过注入实现执行、校验候选及拒绝缺失或冲突实现。
