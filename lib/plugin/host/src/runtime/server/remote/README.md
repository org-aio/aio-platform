# 外部 MCP 与 gRPC 能力

从 `plugins/remote_capabilities.rs` 看装配与生命周期，再看 `provider.rs` 的在线发布、`bindings.rs` 的不可变租户路由及输入输出校验，最后看 `mcp.rs`、`grpc.rs` 的协议适配。`mcp_stdio.rs` 只处理本机进程及受限管道，复用 MCP 的发现、调用和取消。原生插件身份仍是 Dill 中的具体 TypeId；外部服务名、工具名和版本路径只是协议符号。

按需启用 `az-plugin-host/remote-mcp`、`az-plugin-host/remote-grpc`，或用 `remote` 同时选择两者，在宿主组合中选择 `RemoteCapabilitiesPlugin`，并设置 `HostConfig.remote_capabilities`。默认组合在启用 feature 后包含该插件；不配置文件时没有远端能力和连接。出站协议不启用语言实现、入站协议、Wasm 或 Sidecar；脚本宿主须另选语言或装配外部执行器。配置包含未编译的协议时在解析阶段拒绝，不读取描述文件或尝试连接。提供了配置但未启用 feature 或未装配插件时，宿主启动失败，避免静默忽略。

开发宿主与产品宿主通过 `AIO_REMOTE_CAPABILITIES` 指定文件。复制 [示例配置](examples/imports.json)，把租户、地址和凭据环境变量名改为部署配置；凭据只从环境读取，不写入配置、目录或脚本。gRPC 示例描述文件的生成方式为：

```sh
protoc --include_imports --descriptor_set_out=calculator.bin calculator.proto
```

在示例目录执行，或者将 `descriptor_set` 改为相对于配置文件的路径；可以使用完整绝对路径。描述必须包含方法依赖的全部消息。示例 proto 是独立的 `external.calculator.Math`，导入不要求上游实现 AIO 的 Protobuf 服务。

每项导入声明：

| 字段 | 约束 |
| --- | --- |
| `tenant_id` | 明确租户，无通配符；同一租户同一路径只能绑定一次 |
| `capability` | 版本路径、说明、副作用类型和输入输出 JSON Schema |
| `endpoint` | HTTP MCP/gRPC 必填；HTTPS，或固定回环/私网 IP 的 HTTP；无地址凭据、查询参数或片段；gRPC 无路径前缀；stdio 禁用 |
| `credential_env` | 可省略；指定包含 Bearer token 的环境变量名，不转发用户的会话或 Cookie |
| `transport.protocol` | `mcp`、`mcp_stdio` 或 `grpc` |
| MCP `tool` / `output` | 工具名；`structured` 返回 `structuredContent`，`envelope` 返回完整成功结果对象 |
| stdio `command` | 绝对 `program`、绝对 `cwd`、字面 `args` 数组、子进程变量名到宿主变量名的 `env` 映射；禁用 `credential_env` |
| gRPC `descriptor_set` / `service` / `method` | FileDescriptorSet 文件、完整服务名、unary 方法名 |

配置版本为 1，每份配置允许 0 到 64 个绑定，配置和描述文件各最多 1 MiB，单次候选读取总量最多 8 MiB。输入输出使用离线 JSON Schema 校验；只解析 schema 内定义的引用，不取网络或本机文件。不同租户可把同一路径绑定到不同上游，但公开契约必须完全一致。

## 本机 MCP stdio

复制 [stdio 示例](examples/stdio.json)，由部署方安装可执行程序并配置工作目录。宿主不下载软件、不拼接 shell、不转发用户会话；命令和环境来自固定部署文件，租户及脚本不能提交命令。`env` 只保存名称映射，例如 `{"API_TOKEN":"CALCULATOR_TOKEN"}`，值在绑定准备时读取。子进程环境先清空，`PATH`、`HOME` 等也须显式映射；程序需要额外的系统环境时由部署方声明。最多 128 个参数、64 个变量，参数与环境总量不超过 64 KiB。

启动导入和每次调用均创建独立进程，完成握手及契约发现后最多执行一次工具，随后关闭会话并终止进程组、等待直接子进程退出。取消、截止时间、握手失败和插件关闭都走同一任务作用域，清理完成前不释放并发配额。Unix 使用独立进程组，Windows 使用 Job Object；异常 Drop 会再次尝试终止。它是受信任的本机程序连接方式，不提供文件、网络、CPU 或内存沙箱；进程主动脱离 Unix 进程组也不受此机制限制。需要系统隔离时使用现有 Sidecar 容器与监督服务。

stdout 只接收 MCP JSON 消息，单行最多 1 MiB，整场会话累计读取最多 4 MiB；无效消息或超额立即关闭。stderr 丢弃，原始错误行不会写入此适配器的日志。协议编码、消息及握手使用官方 SDK；受限行解码使用 Tokio LinesCodec，避免 SDK 解码错误日志带出原始行。正式配置使用不可变的版本化程序路径，升级时切换配置；绑定快照固定命令和环境，但不会复制或校验磁盘上的可执行文件内容。

## 在线更新与停用

正式宿主就绪后，每 2 秒检查配置及其 gRPC 描述文件的内容摘要。部署方先写入新文件，再通过同一文件系统的原子 rename 替换 `AIO_REMOTE_CAPABILITIES` 指定文件。描述文件建议使用不可变版本路径，最后替换配置；仅修改描述内容也会触发更新。未指定配置路径时不创建观察任务；需要从零在线添加能力时，以 `{"schema_version":1,"imports":[]}` 启动。

更新先在隔离候选中完成文件读取、schema 校验、全部上游连接与 MCP 契约检查，再校验文件是否仍为同一版本，最后在统一能力目录中原子发布。任一检查失败都保留整个旧版本，不会部分导入。每次尝试最多 30 秒，失败以 4、8、16、30 秒退避重试校验，并记录拒绝原因；这些重试只做连接与发现，不执行业务工具。日志记录成功切换的进程内 generation 和内容 revision，不记录凭据。

一次增强执行及其同包子调用固定使用同一份能力快照。更新后新调用使用新绑定，正在执行的调用沿用旧绑定直至结束、取消或最多 30 秒的截止时间；旧连接随最后一个快照释放。删除某租户绑定后，该租户的新发现和调用立即不可用，其他租户不受影响。`imports: []` 停用全部外部能力。回退时原子恢复旧配置及描述文件，同样经过校验后发布。

同一路径的提供者、schema、说明及副作用契约在本进程内不可改变，删除后重新加入也不能改变；修改契约使用新的版本路径。目录最多保留 4096 项历史契约。地址与租户绑定可变化，但部署方仍须保证上游符合业务语义，schema 和工具注解不能证明算法等价。重启读取磁盘上的配置，进程内 generation、历史契约与被拒绝候选不会持久化；不要用重启绕过版本路径约定。无效文件不会在重启时自动恢复为旧配置，部署方需保留可回退版本。

文件观察任务由插件生命周期管理，停止会取消候选校验并禁止后续发布。MCP 的发现和工具请求共用受管理的会话，取消时先发送协议取消通知再关闭会话；有状态服务器的发现请求也纳入关闭验收。开发沙箱不运行正式宿主的后台就绪钩子，仍在启动时导入。没有允许租户修改上游地址的 HTTP 接口；凭据继续使用宿主环境，环境变量值的轮换需要重新启动进程。

MCP 使用官方 SDK 的 Streamable HTTP 或 stdio；HTTP 支持有状态和无状态服务器。启动和每次调用前检查工具目录及输入 schema；`structured` 还要求上游声明完全相同的输出 schema。`read` 要求上游明确标记只读，`idempotent_write` 要求只读或幂等标记；`write` 表示不能保证幂等。注解是契约声明，不是副作用证明。任何类型均不自动重试业务调用，不重新提交交互任务，不因会话过期重放工具；调用后关闭独立会话。

gRPC 复用 Tonic Channel 和 Prost 动态消息，输入按 Protobuf JSON 解码且拒绝未知字段，输出保留普通字段的默认值。64 位整数为字符串、bytes 为 base64、枚举通常为名称，schema 必须遵循 Protobuf JSON 映射；可选字段和 oneof 保留存在性语义。它不自动生成业务 schema，也不提供服务反射或 streaming RPC。

两种语言使用相同的能力路径：

```rhai
fn invoke(input, context) {
    call_capability("/v1/calculator/mcp_double", input)
}
```

```typescript
export function invoke(input: { value: number }) {
  return call_capability("/v1/calculator/grpc_double", input);
}
```

增强包的 `capabilities` 声明直接依赖；样例提供有序 `{path,input,output}` 调用记录，验证不连接真实服务。用户还需 `runtime:<path>` 权限；Wasm/Sidecar 的包声明与交互用户授权求交集。声明外部能力、能看到目录或管理增强都不自动获得执行权。租户路由、身份、授权和截止时间来自宿主，脚本不能指定上游地址或改写这些数据。

调用沿用整条脚本链的截止时间，gRPC 入口传入的较短期限继续传播到上游 `grpc-timeout`。关闭或取消等待会丢弃协议工作，不把仍执行的脚本计为已释放。传输响应最多 1 MiB，最终 JSON 仍限 64 KiB；MCP HTTP 在解析前限制 JSON、分块响应和未终止 SSE 的累计字节，并拒绝重定向。每个 MCP 绑定最多 4 个调用，取消清理继续持有配额；握手限 5 秒，取消通知、会话关闭和 stdio 进程等待各限 2 秒，清理任务由插件收容并在停止时等待。取消不能撤销已经产生的外部写入，没有自动补偿。

运行真实 PostgreSQL、MCP SDK 和独立 gRPC 服务的验收：

```sh
cargo build -p az-plugin-host --no-default-features --features remote --example mcp-stdio-fixture
AIO_ENHANCEMENT_TEST_DATABASE_URL=postgresql://test@127.0.0.1:5432/remote_test \
AIO_REMOTE_TEST_CREDENTIAL=remote-test \
AIO_MCP_STDIO_TEST_EXECUTABLE="$PWD/target/debug/examples/mcp-stdio-fixture" \
AIO_MCP_STDIO_TEST_TOKEN=stdio-test \
AIO_MCP_STDIO_HOST_SECRET=must-not-inherit \
  cargo test -p az-plugin-host --no-default-features --features remote,mcp,grpc,rhai,typescript \
  --lib protocols::tests::remote -- --ignored --test-threads=1
```

覆盖双语言、双协议、同路径不同租户绑定、候选无副作用、恢复、权限撤销、失败不重试、契约漂移、超限响应、零值 Protobuf JSON、短超时和取消传播。在线更新验收将真实脚本暂停在第一次远端调用中，切换绑定后证明旧脚本的第二次调用仍访问旧上游，新请求通过 MCP/gRPC 入口访问新上游，另一个租户的绑定保持独立；还覆盖空配置、整批拒绝、删除后契约冲突和描述文件更新。

Linux stdio 验收使用真实 SDK 子进程及其后代，覆盖双语言、环境与字面参数、恶意/超大输出、通知洪泛、异常退出、握手/发现挂起、取消、满额和插件停用；直接子进程必须被 wait，后代不得继续运行。还覆盖整批配置预检、失败保留旧绑定及运行中的脚本固定命令快照。macOS/Windows 的实际进程验收、流式 RPC、MCP 交互/后台任务和原生宿主自更新仍待完成。
