# 宿主脚本增强

`controller.rs` 是管理员 API，`store.rs` 统一负责候选验证、租户版本、激活和执行。`EnhancementServicePlugin` 初始化服务并注入同一个 `ScriptRuntime`；WIT、process broker、HTTP、MCP 和 gRPC 都使用它。PostgreSQL 是活动版本的权威源；内存只缓存已验证的不可变代码。每次调用读取一次活动版本，随后完整使用该版本。切换前已接收的调用可以完成，响应返回实际执行的 revision。

只有当前租户的 `plugin:manage` 会话能够保存、查看和激活版本，请求不能指定其他租户。插件只获得清单声明的版本路径调用权，不能发布或激活脚本。以下 API 由 `EnhancementsPlugin` 提供，需要数据库与脚本执行器，不需要配置 Component 存储：

| API | 行为 |
| --- | --- |
| `GET /api/runtime/enhancements` | 当前 revision 和 generation |
| `GET /api/runtime/enhancements/revisions` | 最近 50 个候选版本，时间为毫秒 |
| `POST /api/runtime/enhancements/revisions` | 编译并执行包内样例，成功后保存候选 |
| `GET /api/runtime/enhancements/revisions/{revision}` | 当前租户的完整候选包 |
| `PUT /api/runtime/enhancements/active` | `{ "revision": "摘要", "expected_generation": 0 }` 激活；revision 为 null 时停用 |
| `POST /api/runtime/enhancements/rollback` | 同样的请求结构，仅允许回到本租户曾激活的版本 |

普通升级必须保留已有路径和样例，并用候选实现重新执行旧样例。回滚是显式恢复历史版本，可能使新版本新增的能力暂不可用。版本指针和审计事件在同一事务提交；代数冲突返回 HTTP 409，不能用重试覆盖别人的发布。重新创建宿主后从数据库恢复，执行前仍需重新编译和验证。

每个宿主实例最多同时进行 4 个增强编译、验证或执行任务。任务进入 blocking pool 前就取得配额；等待方取消后，执行中的调用链收到取消信号，配额随实际工作线程结束才释放。代码缓存最多 16 个包，无租户业务状态。

Rhai 和 TypeScript 通过 `call_enhancement(path, input)` 组合包内增强；声明、授权与预算见[增强执行器](../../../../../enhancement/README.md)。服务把原始调用者的能力清单传入整条调用链，不给子调用追加管理员权限，也不为每一层重新申请并发槽或读取活动版本。升级增加依赖不会自动给旧插件增加权限。

测试使用独立本机数据库：

```sh
AIO_ENHANCEMENT_TEST_DATABASE_URL=postgresql://test@127.0.0.1:5432/kernel_test \
  cargo test -p az-plugin-host --features server enhancements::tests -- --ignored
```
