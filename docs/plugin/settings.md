# 插件设置

v2 插件在清单声明自己的设置页，安装后自动贡献到“账户菜单 → 设置中心”的插件分组。分组标题下以小字显示“来自插件（插件 title）”，选中后直接在设置中心显示该插件的设置内容。已安装插件的市场详情保留快捷入口。业务页面统一放入工作空间，设置页不进入业务菜单或账户菜单。

```toml
schema_version = 2
[plugin]
settings_page = "settings"
[plugin.runtime]
host_version = ">=2026.9.18"
# artifact、frontend 等使用插件原有声明
```

组件 describe 或进程 /aio/describe 同时声明 id=settings 的页面，surface 使用已有 fullscreen，entry 指向 settings.html，scene 为空、menu_path 为空。宿主校验页面确实存在并按原有权限过滤，不需要升级 WIT enum。页面在设置中心内容区的独立沙箱 iframe 中挂载，通过相同 AIO Web SDK 调用插件自身接口，租户/用户身份仍由宿主注入。只挂载选中分组；停用、卸载、撤权及版本变化同步更新分组和页面实例。分组来自运行时目录，无需修改设置中心或登记插件专用路由。

设置表单、校验和持久化归插件；平台不建立通用明文 Key 表。不把密钥塞进清单、URL、catalog 或前端状态。插件可申请 database/cryptography，按租户及用户/服务配置范围加密，GET 仅返回 hasSecret。留空保留、显式清除由插件接口定义。

需要第三方 JSON API 的原生 v2 process 同时声明完整 HTTPS 地址：

```toml
[plugin.runtime.process]
http_endpoints = ["https://api.tavily.com/search"]
```

管理员在 AIO_PROCESS_HTTP_ENDPOINTS 批准同一完整地址。调用 Unix broker 的 POST /egress/http，以 x-aio-endpoint 选择地址，x-aio-token 使用实例票据，Authorization 可携带插件自己的密钥；只接受 JSON POST，不允许任意方法、子路径和重定向。宿主复核活动安装版本，限制并发、30 秒请求时间与 512 KB 响应。不得向浏览器暴露 broker 票据。模型仍使用 endpoints 与专用 /egress、/egress/models。

旧包省略这两个新字段时序列化保持原形；现有 WIT 四种页面类型不变。示例实现见 aio-plugin-agent 的独立 settings.html 与 tools/web-search 接口。
