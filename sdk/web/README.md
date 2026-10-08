# Browser Bridge SDK

`lifecycle.js` 由正式宿主先行注入。宿主以入口查询参数 `__aio_prepare=1` 显式创建后台预备页面，准备阶段仅初始化 UI 和资源；`aioPlugin.request/json` 等待宿主首次激活后才启动请求及超时计时。普通挂载不等待预备握手，避免在滚动发布或开发预览中阻塞可见页面。宿主通信桥再次检查激活状态，不能通过直接发消息绕过。未激活队列最多 16 项，租户暂停时拒绝待执行调用。已访问页面切走后保留实例；权限和版本撤销由宿主销毁挂载。

`guest.js` 为隔离 iframe 提供二进制请求与 JSON 辅助接口。`host.mjs` 绑定单个窗口，不读取插件 DOM，不向插件暴露 Cookie 或管理票据。

`request` 和 `json` 接收 `/graph?spaceId=...` 形式的服务地址，统一用 URL 解析器分离 v2 的 `path` 与 `query` 字段。也可显式传递 `request.query`，但不能同时在两个位置提供查询参数。地址必须属于当前插件，拒绝外部地址和片段。

`wasm.js` 由宿主在插件入口之前注入，统一先下载、后编译 Wasm。通过读取响应副本解除隔离页面中流式编译对公网下载的反压，原响应及编译参数仍交给浏览器原生 API 校验；不放宽 iframe、CSP 或请求权限。加载期间会暂存模块字节，不持久缓存带票据的资源。运行 `node --test sdk/web/*.test.mjs` 验证桥接和加载行为。

`aioPlugin.copy(text)` 请求宿主复制文本，要求挂载时显式授予 `{clipboard: true}`、页面具有焦点及当前用户手势。文本有长度上限，拒绝或超时返回错误；不会把剪贴板内容转发给插件服务或模型。该授权不提供读取剪贴板能力。

此 SDK 使用 v2 消息；正式产品会话撤销、全屏和账户挂载仍在迁移中，不能把开发预览服务用作公网宿主。

## URL 导航与滚动

宿主在两种 iframe ABI 的入口前注入 `navigation.js`，`aioPlugin.navigate(fragment, { replace })` 更新外层 URL，`aioPlugin.onNavigationChange(listener)` 恢复外层浏览器历史。hash router 自动同步；memory router 需显式在回调中恢复，并把业务筛选编码入自己的 hash 契约。模块沿用当前挂载票据、父窗口和 opaque-origin 校验，不暴露宿主 DOM 或 Cookie。

窗口滚动和 `data-url-scroll="稳定名称"` 标记的内部容器同步为外层有界 `scroll` 参数，连续滚动只 replace。刷新或复制链接到新浏览器上下文后，在页面内容出现时恢复；要跨数据变化精确定位，使用稳定记录锚点，而不是承诺像素偏移永远精确。

原生 hash 变化已经创建 iframe 的联合浏览器历史，宿主同步时只 replace，避免一次后退停在重复视图。逐节点滚动暂存于 iframe 的 history.state，保留已有对象型 router state；当前可分享位置仍写在外层 URL，重新打开不依赖这份本地历史。显式调用 aioPlugin.navigate 的导航由宿主创建历史节点。

`aioPlugin.download(name, bytes, mime)` 让宿主下载浏览器内编辑产生的文件，不上传内容。要求当前用户手势、宿主焦点和可见 iframe，最大 16 MiB，拒绝带路径或控制字符的文件名。`mountBridge` 可用 `{download: false}` 禁止下载；不放宽 iframe sandbox/CSP。返回 204 表示已发起浏览器下载，不表示用户已经将文件保存到磁盘。

`aioPlugin.onFileDrop(listener)` 注册当前插件的文件拖入接收器，返回注销函数；插件内部或嵌套设备视图收到带 `Files` 的 `dragenter` 时调用 `aioPlugin.fileDrag()`。宿主以 `mountBridge` 的 `fileDrop` 回调确认页面激活、可见和当前挂载票据，短暂将 iframe 设为 inert 以接收真实 drop，随后恢复原实例。目录由真实来源的宿主读取，传给插件的是 File、目录名称、相对目录项和拖入坐标；不传原始绝对路径、目录句柄或宿主票据，也不修改沙箱。

同一次拖入最多 4096 个文件/目录项、32 层相对路径，单文件最多 128 MiB、合计 512 MiB；保留空目录和分页读取结果。读取失败向当前接收器报告错误。挂载票据变化、重新注册、卸载或后续拖入使旧读取结果失效。宿主只读取用户实际拖入的对象，设备上传与保存仍由插件自己的协议负责。

## 配对设备视图

Component v2 插件声明 `worker_capabilities=["codex.web"]` 后，可调用 `aioPlugin.deviceView({operation:"list"})` 读取本人设备，调用 `{operation:"open",device:"UUID",route:"/local/thread-id"}` 得到短期 `{id,src}`，以及 `{operation:"close",id}` 关闭视图。`route` 仅支持应用 pathname，不带查询、片段或凭据。

将 `src` 放入 `sandbox="allow-scripts"` iframe。消息协议 `aio:device-view@1` 提供 ready、error 和 route 通知；接收方必须验证 `event.source` 是该 iframe。视图地址不是可分享链接，不得写入外层 URL；仅将设备 UUID 与应用 pathname 通过现有 `aioPlugin.navigate` 保存。设备凭据、账户、原生运行时和文件路径由设备保管。

设备撤权、登录失效、插件停用/更新、30 分钟视图过期或连接中断要求重新建立视图，旧写请求不重放。其他插件和旧 PageDefinition 入口不能使用此接口。宿主通道与权限说明见 `lib/plugin/host/src/generated/worker_webview/README.md`。
