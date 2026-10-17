# 系统整包的宿主管理能力

`ComponentManagementPlugin` 让部署方选定的 v2 整包通过公共宿主契约管理插件目录和安装。Wasm、Sidecar 与挂载前端复用相同的管理授权和路由分发，不持有管理员 Cookie，也不通过本机 HTTP 回环伪造身份。

## 部署声明

管理包必须同时满足清单和部署信任：

```toml
[plugin]
required_permissions = ["plugin:manage"]

[plugin.capabilities]
management = true
```

```rust,ignore
runtime_plugins.push(Arc::new(ComponentManagementPlugin(vec![TrustedManagementComponent {
    git: "https://github.com/example/manager.git".into(),
    digest: expected_bundle_digest,
}])));
```

`digest` 是打包器输出的规范整包 SHA256，覆盖来源、版本、清单及全部文件。仅在验证完整发布包后记录来源；开发目录不能自报摘要取得信任。Git 来源必须是无凭据、查询和片段的 HTTPS 地址，声明最多 128 项，不接受重复项。新版本需单独加入声明；安装包不能修改这份部署配置。

`aio-idea` 在 `AIO_RUNTIME_COMPOSITION` 指向的 JSON 中配置：

```json
{
  "schema_version": 1,
  "trusted_management_components": [
    {"git": "https://github.com/example/manager.git", "digest": "<64 位小写整包 SHA256>"}
  ]
}
```

默认没有可信管理包。声明变更经过已有组合恢复记录，在线重建组件环境；撤去信任暂停对应安装，保留摘要、启用意图、激活代数及回滚排除记录。恢复信任后按原记录恢复，不隐式升级。首次安装另用 [ComponentBootstrapPlugin](component-bootstrap.md)；撤去信任不会被首次引导重新授权。`identity_provider` 仍不开放。

## 三种调用入口

前端使用宿主注入的浏览器 SDK：

```javascript
const response = await aioPlugin.manage({
  method: 'GET', path: '/api/runtime/catalog', headers: [], body: new Uint8Array(),
});
if (response.status !== 200) throw new Error(new TextDecoder().decode(response.body));
const catalog = JSON.parse(new TextDecoder().decode(response.body));
```

请求通过当前挂载票据进入 `/api/runtime/components/{token}/manage`。宿主检查页面、租户、会话、版本、代数和实际权限。前端入口不占用插件执行器，因此可以升级、回滚、停用及卸载自身；成功的管理写操作触发宿主目录刷新。暂停或撤销挂载后旧票据失效，恢复使用新票据。设备视图与开发预览不开放此能力。

Wasm 使用现有 WIT `management.invoke`，Sidecar 向其宿主 broker Unix socket 的 `POST /management/invoke` 提交与组件 HTTP 请求相同的 JSON：`method`、`path`、`query`、`headers`、`body`。Sidecar 同时传进程 `x-aio-token` 和当前宿主登记的 `x-aio-context`；响应为 `status`、`headers`、`body`。进程授权固定为启动该版本时的快照，不继承后续包新增能力。

管理调用必须来自本插件仍存活的真实交互请求。每次调用重查成员、会话与 `plugin:manage`；拒绝后台、服务会话、设备、跨租户、伪造或过期上下文。跨插件派生上下文不能转授管理能力，普通插件不能借可信代理提升权限。

执行器内的同步调用不能变更任何插件生命周期，避免与升级排空及全局变更锁互相等待。这些动作应由上述前端入口执行；后台持久管理任务尚未提供。

## 开放范围

管理分发复用正式路由及其发布、租户、设备检查，额外限制方法和路径：

- 查询目录、市场详情/图片、版本、包下载、发布任务、插件事件、工具访问/详情/设备和工作任务。
- 安装、启用、停用、回滚、卸载、隐藏/显示菜单，重试交付任务。
- 注册、更新、删除工具和安装工具到设备，删除市场来源。

不开放包上传发布、凭据、身份、内部交付接口或任意服务代理。请求只接受 `content-type`、`accept`，不能注入身份头；请求体最多 8 MiB、响应体最多 32 MiB。返回仅保留内容类型和下载文件名相关头。浏览器大包下载另走下面的原始响应通道。

## 前端交互动作和大包下载

当前可见管理页在用户点击时可调用：

```javascript
await aioPlugin.downloadPackage(revision);
await aioPlugin.openSettings(pageId);
await aioPlugin.installTool(toolId, version);
```

这些动作只接受语义参数。宿主通过当前挂载的 `/action` 核验相同的部署信任、实时权限和版本；SDK 不接受任意 URL，不向插件提供 Cookie。设置页必须存在于当前用户的已授权设置目录；本机安装必须是仍启用的工具契约中已收录的版本，由宿主生成规范 `aio://install/...`，助手继续展示计划并等待本机确认。

`downloadPackage` 触发浏览器访问 `/api/runtime/components/{token}/packages/{revision}`。下载入口再次核验登录、租户、活动挂载、版本和部署信任，然后复用正式包下载处理器。压缩包使用原始 HTTP 响应，不经过 JSON 或消息数组，因此不受管理响应的 32 MiB 限制，仍受整包总大小限额约束。返回成功表示已启动浏览器下载，不代表文件已保存。

桥接拒绝隐藏、无手势、已暂停或已卸载的页面。等待授权期间暂停页面会取消操作；续期使用新票据。管理包不能借此重新开放被部署方移除的工具服务。

## Markdown 图片

`aioPlugin.managementImage(url)` 使用当前管理挂载取回图片字节。站内仅允许正式市场文档图片路径，支持 URL 编码的中文和空格；交付服务停用时仍拒绝读取。外部图片仅接受无凭据的公网 HTTPS 443，逐次验证重定向和 DNS，绑定已验证地址，不携带宿主凭据或使用本机代理。图片限制为 10 MiB，总等待最多 15 秒。

插件把响应体转换为 Blob URL 后交给共享 Markdown 组件，文档切换和卸载时释放资源。每次读取由宿主使用当前票据授权；不会把旧票据长期保存在图片地址中，网络等待期间撤权或暂停也不会返回图片。此能力不授予任意网络代理或站内 API 访问。

## 验证与实现入口

先读 `plugins/component_management.rs` 的部署信任，再读 `components/management.rs` 的实时授权，最后读 `management_dispatch.rs` 的公共路由分发。`components/frontend_management.rs` 与 `components/process/management.rs` 只负责各入口的上下文构造。

`tests/component_execution/management.rs` 使用真实 HTTP/PostgreSQL 与替代执行器，覆盖无信任拒绝、新版单独授权、自升级/回滚/停用/卸载、撤权、撤信任和重启恢复。`components/management_tests.rs` 使用真实 Wasm 和 Sidecar Unix broker，监督器为测试夹具，验证上下文归属、不可转授、实时撤权和旧票据失效。后者需要 `AIO_COMPONENT_TEST_DATABASE_URL`、`AIO_TEST_HEALTHY_COMPONENT` 及专用进程目录，随 CI 执行。

`node --test sdk/web/*.test.mjs` 验证消息边界；`tests/browser/component-lifecycle.cjs` 用真实 Chromium 和模拟挂载 API 验证正式 SDK、独立管理入口、暂停/续期票据与目录刷新。它不证明真实市场 UI 或 Docker 隔离已验收；市场系统包仍待迁移。

`tests/component_execution/management_actions.rs` 通过真实 PostgreSQL/HTTP 下载超过 32 MiB 的有效整包并逐字节核对，覆盖设置目录、工具版本、服务停用、会话和权限撤销；浏览器用例另覆盖真实下载、设置事件与待处理动作取消。Rust 客户绑定见 `lib/plugin/guest`，从唯一 WIT 定义生成，正式生命周期夹具使用该 crate 编译。
