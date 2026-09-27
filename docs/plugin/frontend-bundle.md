# 前后端联合二进制包

v2 已实现共享整包校验、前端隔离挂载和会话调用桥。实际宿主配置、租户安装以及各框架的 Web/Desktop 制品仍需分别验收；打包成功本身不证明界面已经可用。

## v2 入口

声明 `schema_version = 2` 的清单使用 `az-plugin-bundle::Bundle`，包含运行时 artifact、前端目录和可选数据库迁移。后端通过 `/aio/describe` 或 Component `describe` 返回页面描述，每个页面的 `entry` 引用前端目录内的 HTML 文件。宿主将来源、租户、登录用户、活动版本和页面权限绑定到挂载授权，前端通过 `aioPlugin` 调用桥访问插件。

v2 允许最多 4096 个文件、64 MiB 文件总量和 96 MiB 编码包。具体字段及发布流程见 [publish.md](publish.md) 的 v2 整包章节。以下 `kind`、`/aio/definition` 和 PageDefinition 示例属于旧版联合包契约。

## 同仓交付

一个功能仓库内包含 `frontend/`、`backend/` 和 `shared/`。三者用各自工具链构建，最终只有一个 `.aio-plugin` 包和一个发布版本。仓库不必提交 `dist/`，平台不在安装时调用 Cargo、Gradle、npm 或脚本。

```toml
[plugin.runtime]
kind = "wasm-component"
artifact = "dist/backend.wasm"

[plugin.frontend]
path = "dist/web"

[[plugin.subplugins]]
id = "orders"
pages = ["orders"]
routes = ["orders"]
```

前端资产集合独立于后端运行形态，同样可以配合 `process` 或纯静态 `page-definition`。Rust、Kotlin、TypeScript 共用此文件协议，不把框架、语言或字符串插件身份写入运行扩展注册。框架自己的编译输出属于程序产物，不能写入正式 `PageDefinition` 的页面定义字段。

## 页面入口

后端的 `definition()` 或 `/aio/definition` 返回的正式页面定义只引用包内 HTML 入口：

```json
{
  "id": "orders",
  "label": "订单",
  "icon": "panels-top-left",
  "scene": { "id": "workspace", "label": "工作区" },
  "body": { "kind": "frontend", "entry": "index.html" }
}
```

`entry` 相对于 `plugin.frontend.path`，不能是外站 URL、绝对路径或路径穿越。多个页面可以指向同一个前端入口。导航、权限和页面身份仍由宿主的 `PageDefinition` 决定，编译产物中的 UI 代码不能自行新增宿主路由或权限。

## 打包不变量

`aio plugin package . --git <HTTPS 来源> --version <SemVer>` 收集声明目录中的普通文件，校验入口并将前后端作为格式 2 的同一个包编码。`verify()` 返回后端字节以及排序的前端路径到字节集合，宿主只能在校验完成后写入版本目录。

- 前后端字节合计最多 32 MiB，前端文件最多 256 个，单一路径最多 240 字节。
- 路径使用 ASCII 字母、数字、下划线、连字符、点和 `/`；拒绝空路径段、隐藏文件、设备名、反斜杠、URL 转义和符号链接。
- 文件不能同时占据另一个文件的父目录，大小写不同但等价的文件名也被拒绝。
- 前端不得覆盖 `aio-plugin.toml` 或后端 artifact。
- 每个前端文件有 SHA-256；文件路径与摘要均参与完整包的内容 SHA-256。前端替换、后端替换、新增或删除文件都会更改包版本。
- PostgreSQL 保存的是完整可下载程序包及正式页面定义，不把渲染时产生的 HTML、CSS、UiOp 或 Dioxus Element 当作页面状态保存。

## 宿主接入边界

宿主把前端资产、后端实例、路由和租户绑定视作一个版本，完成共同激活与回滚。前端代码不得获得宿主 DOM、Cookie 或任意同源请求能力；通信桥从会话与活动页面解析租户、来源、版本和允许的服务路由，不能接受前端自报上下文。v2 已实现这些校验；每次实际安装仍应检查入口、资源和业务调用，避免把市场发布当作浏览器验收。
