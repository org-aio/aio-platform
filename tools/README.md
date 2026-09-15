# CLI 市场

CLI 与宿主插件显示在同一个插件市场中，标签包含 `cli`；安装目标为访问者的电脑。条目通过 `registry/` 中的 JSON 描述支持平台、依赖、安装、检测与卸载命令，不打包第三方 CLI。

宿主把内置条目和 `AIO_TOOL_REGISTRY_DIR` 下的 JSON 导入 PostgreSQL。发布新版本须增加新 JSON，旧版本保持不可变。网页只负责选择，命令由用户电脑执行。
