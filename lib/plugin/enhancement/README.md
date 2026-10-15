# 脚本增强执行器

`az-plugin-enhancement` 执行 Rhai 和 TypeScript 宿主能力。它不依赖 HTTP、数据库、UI 或业务插件，宿主负责版本存储、授权和激活。

先看 `src/model.rs` 中的版本包，再看 `PreparedPackage::prepare` 和 `invoke`。每个路径必须包含协议版本，例如 `/v1/text/normalize`。Rhai 导出 `fn invoke(input, context)`，TypeScript 导出 `export function invoke(input, context)`；输入输出均为 JSON。TypeScript 使用 Oxc 转译及 QuickJS 执行，不需要 Node 或外部 npm 包。

候选包必须通过全部契约样例，普通升级还会重放旧包样例。样例验证不能证明任意输入行为相同，破坏性变化应使用新版本路径。每次调用重新创建脚本作用域，不共享跨租户全局变量。脚本不获得文件、网络、进程、环境变量、模块加载和宿主管理接口。操作数、时间、递归、源码和输入输出均有上限，QuickJS 另有堆与栈限制。

这提供可回滚的运行时算法增强，不代表任意 AI 生成代码已经可信，也不能用脚本替换原生执行器、修改权限或自行突破配额。AI 可以生成并提交候选，是否激活由宿主现有权限边界决定。

验证：`cargo test -p az-plugin-enhancement`。
