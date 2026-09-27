# 远端命令

主入口先执行内置命令，其余 argv 由本模块交给 vibecli 服务。命令树、参数、帮助和逻辑由服务端解析；更新服务中的 catalog 或命令逻辑后，已连接的主体 CLI 无需重新发布。

```sh
aio vibecli connect http://127.0.0.1:8787/api/runtime/vibecli
aio --help
aio greet --name example
aio greet --help
aio vibecli disconnect
```

`connect` 验证 catalog 后保存当前目录 `.aio/vibecli.json`：

```json
{"schema_version":1,"transport":"direct","base_url":"http://127.0.0.1:8787/api/runtime/vibecli"}
```

`AIO_VIBECLI_URL` 覆盖项目端点。可选的 `AIO_VIBECLI_TOKEN` 仅从环境读取，通过 `Authorization: Bearer` 发送，不写入配置。端点允许 HTTPS 或回环 HTTP，不允许凭据、query、fragment；路径前缀会保留。

## AIO 宿主连接

通过密码标准输入登录后，将项目连接到已安装的 vibecli 组件：

```sh
aio vibecli login https://aio.example.com --account example --password-stdin
aio vibecli connect https://aio.example.com --source 37e77f55-4210-4b73-a974-9d19d8c99f21 --project 47937056-ad6e-4d0b-b09b-91c2cd2f53a4
aio greet --name example
```

登录请求使用 `POST /api/auth/login` 的 `{account,password}`，只保存响应中的 `aio_session`。专属会话位于 `~/.config/aio/vibecli-session.json`，可用 `AIO_VIBECLI_SESSION_FILE` 覆盖；Unix 权限为 0600。项目配置只保存 `schema_version`、`transport:"aio"`、`origin`、`source`、`project`，不保存 Cookie、密码或 token。

AIO 端点必须是纯 origin。每次 catalog/invoke 使用同一会话 Cookie 先挂载 `component:<source>:vibecli`，取得 v2 grant 后通过 `/api/runtime/components/<grant>/request` 访问 `/api/cli/<project>/catalog|invoke`；组件请求 body 使用 UTF-8 JSON 字节数组。grant 不保存或缓存；已过期会话需要重新登录。

## 协议

`GET <base>/catalog`：

```json
{"schema_version":1,"revision":"r1","commands":[{"path":["greet"],"description":"问候"}]}
```

`POST <base>/invoke`：

```json
{"argv":["greet","--name","example"]}
```

响应：

```json
{"stdout":"hello example\n","stderr":"","exit_code":0,"revision":"r1"}
```

CLI 保留 stdout、stderr 和 0..255 退出码。revision 可选；每次顶层帮助读取当前 catalog，每次远端命令直接调用 invoke，不缓存命令目录。每个 HTTP 请求总超时 30 秒，响应上限 1 MiB，不跟随重定向。服务失败直接返回错误，不执行本机替代命令；顶层帮助仍展示本地用法并提示远端错误。

模拟服务验收：`cargo test -p az-aio-cli --test runtime_commands`。
