# 从插件市场安装本机 CLI

CLI 与宿主插件共用一个市场，条目包含 `cli` 标签及结构化安装描述。点击 **安装到本机** 使用 `aio://install/<id>?version=<SemVer>` 唤起本机助手。普通插件即使带 `cli` 标签，也不会执行本机安装。

## 首次使用

安装包含本功能的 AIO CLI 后，在终端执行：

```sh
npx -y @zjarlin/aio helper install
```

macOS 注册用户目录中的 AIO Helper.app，Windows 注册 HKCU，Linux 注册 xdg desktop entry。助手保存独立二进制，npx 缓存清理不影响链接。后续更新 CLI 后重新执行此命令更新助手。Linux 桌面需要 xdg-mime 和可用终端；macOS 需要系统自带的 osacompile、codesign。

```sh
npx -y @zjarlin/aio tool install codex-model-sync --version 0.1.4
npx -y @zjarlin/aio tool list
npx -y @zjarlin/aio tool uninstall codex-model-sync
npx -y @zjarlin/aio helper uninstall
```

安装前会展示计划，必须在本机终端输入 `yes`。链接不能传入命令、来源地址或自动确认标记。浏览器可能询问是否打开 AIO Helper。

## 发布 CLI 条目

按照 `tools/registry/codex-model-sync-0.1.4.json` 创建 JSON，声明平台、依赖、安装、检测和卸载命令。在宿主设置 `AIO_TOOL_REGISTRY_DIR` 指向 JSON 目录，启动时验证并导入 PostgreSQL。市场显示每个工具最高 SemVer；各历史版本仍可通过固定链接获取。同 ID、同版本不会覆盖，变更安装步骤必须递增版本。

正式描述接口为 `GET https://aio.addzero.site/api/runtime/tools/<id>/<version>`，只读、公开、不接收命令。助手只请求官方 HTTPS 来源且不跟随重定向。AIO 不镜像或重打包第三方 CLI，安装描述调用其原生分发方式。

## 状态与恢复

安装步骤开始前保存原始描述；安装和检测都成功才记为 installed。安装失败保留记录，卸载使用保存的版本，先恢复配置再移除包。卸载失败保留进度，重试从下一未完成步骤继续。各工具通过当前用户目录中的锁串行操作。

网页无法直接读取本机安装情况，因此 CLI 不计入宿主的“已安装”。查看本机结果使用 `tool list`，不会把点击链接当作安装成功。首次发布需要同时交付宿主、市场前端和新版 npm CLI，旧 npm 版本不包含 helper/tool 命令。

## 边界

这一版提供安装、失败重试、原版本卸载和协议注册。更换版本前必须先卸载原版本，防止覆盖原始恢复描述；依赖缺失时提供修复说明，不自动安装整套包管理器。安装步骤退出成功只能代表条目声明的检测通过，第三方工具的配置恢复语义由其卸载命令负责。
