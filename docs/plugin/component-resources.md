# 插件资源流

文件、导出和图片属于插件业务。宿主提供有界传输、身份与安装生命周期检查；不把文件目录、图床 token 或对象分块规则写进内核。

## 有身份的下载

iframe 在真实点击事件里调用：

```js
await window.aioPlugin.downloadFrom('/files/file-id', '报告.pdf', 'version=2');
```

路径必须是插件内的完整绝对路径，查询串单独传入。SDK 不接受外部 URL、编码路径、遍历或文件名中的路径分隔符。宿主桥检查不透明 iframe 来源、当前手势、焦点、可见性及页面活动状态，立即把下载交给浏览器；返回 204 仅表示请求已交出，不表示文件已经保存。

浏览器访问 `/api/runtime/components/{mount-token}/download?path=...&name=...`。服务端重新验证真实 Cookie、挂载、页面权限、整包 `required_permissions`、版本及激活代数，沿用统一执行器的授权分发。票据只是页面绑定，不能独立代替登录。响应固定为 attachment/octet-stream，不透传插件的 Cookie、重定向或任意响应头。

原生进程使用既有 `Host::protect` 和 `Caller`。`ComponentExecutor::handle_stream` 接收同一 `RequestContext`；默认实现包装原有有界响应，Sidecar 实现使用真正的 HTTP 字节流。Wasm 默认实现仍受其原有响应大小限制。

## 匿名资源

整包必须明确声明允许公开的 GET/HEAD 路径、MIME 类型和字节上限：

```toml
[[plugin.public_routes]]
path = '/i/{token}'
content_types = ['image/png', 'image/jpeg', 'image/webp', 'image/gif']
max_bytes = 104857600
```

只读笔记页或静态展板可以声明静态 `text/html` 并设置 `inline = true`，让浏览器直接渲染而不是下载。内联只允许静态 HTML 或图片；宿主仍发送无脚本沙箱 CSP（`default-src 'none'`、仅内联样式和 `data:` 图片），页面不能加载脚本、外部请求或取得同源权限。不要内联由用户输入直接拼接的 HTML。

最多 16 个完整路径模式；禁止通配前缀、重叠路由、重复参数和重复类型。运行时公共入口为 `/api/runtime/components/public/{source}/{tenant}/i/{token}`。升级沿用来源、租户和业务 token；可读性仍由当前已激活整包及它的清单决定。业务负责生成不可猜测的分享 token 和按租户查找对象；不能把匿名入口挂到列举、修改或管理路由。

进程启动配置提供 `public_routes` 和 `public_base`。`Host::protect_public(router)` 校验启动时固定的入口令牌、租户、公开标记和声明路径，注入只有租户字段的 `PublicCaller`；用户、会话、交互上下文和浏览器 Cookie 不会转入匿名调用。公开入口不记录用户，也不能凭匿名请求调用受交互授权保护的宿主能力。

匿名响应只接受声明的 MIME 类型及正常资源响应，不转发 Cookie、重定向和 CORS 授权。图片可以外嵌；其他类型强制附件。所有资源带 `no-store`、`nosniff` 和限制脚本、嵌入的 CSP，避免图床内容取得主站权限。不存在、停用或不匹配的资源返回 404；配额耗尽返回 429。

声明公开图片类型的整包，其前端 `img-src` 允许当前来源、当前租户的公开资源前缀，便于展示自己的图床图片。其他插件和租户不在该许可内；`script-src`、`connect-src` 和 `frame-src` 不扩大，未声明公开图片的包也不增加该来源。

## 传输与撤销

每次读取最多 128 MiB，公开资源还受清单中更小的上限约束；总截止时间 120 秒。公开读取共用 16 个并发槽；有身份下载共用前端请求槽，槽位覆盖整个响应体。只接受 GET/HEAD，拒绝响应长度重复、超限、截断和越界；没有 Content-Length 时也累计字节数。

公开资源在开始、获取响应后及传输期间复核安装摘要和激活代数。有身份下载持续复核会话、页面及实时宿主权限。检查间隔最多 500 ms，传输已有缓冲可能仍在网络中。插件停用、更新、页面撤销、用户撤权或退出会终止后续读取。公开资源不依赖某位用户会话，因此用户撤权不会取消已明确公开的图片。

字节流保留背压，每个下游队列只缓存一块至多 64 KiB 的数据。上游执行器负责自己的单帧分配；Sidecar 使用 reqwest 的流式读取。传输任务加入 `ComponentsPlugin` 的任务作用域，客户端关闭、截止时间和插件取消会释放上游及并发槽，即使下游已停止轮询 Body。

## 验收边界

- `az-plugin-contract` 与 `az-plugin-process` 测试验证声明歧义、方法/路径限制、租户绑定、重复身份头及匿名身份隔离。
- `component_executors::catalog::streaming` 使用替代执行器、文件目录夹具和真实 HTTP 验证 100 MiB 下载、真实挂载、权限撤销、停用断流及并发槽回收；该目录仅是测试替身。
- `sidecar_resources_keep_public_and_authenticated_ingress_separate` 使用独立 PostgreSQL、真实 Unix HTTP 和原生 SDK 验证 100 MiB Sidecar 读取及身份边界。
- `tests/browser/component-lifecycle.cjs` 使用正式宿主脚本、SDK 和真实 opaque iframe，验证 100 MiB 浏览器下载、暂停拒绝及续期票据；服务端票据为夹具，不能替代上一项真实 HTTP 授权验收。

这些是文件插件独立交付的运行时前提。文件插件的分块上传、旧数据迁移、独立前端及产品静态依赖移除仍需单独完成和验收。
