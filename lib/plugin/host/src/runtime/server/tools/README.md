# CLI 市场条目

同一市场返回宿主插件与 CLI 安装描述。CLI 由 `cli` 字段识别并带 `cli` 标签，不能送入宿主安装接口。本模块只保存和分发描述，不运行命令。

PostgreSQL 保存每个不可变版本。内置条目和 `AIO_TOOL_REGISTRY_DIR` 下的 JSON 在启动时导入；同 ID、同版本不会覆盖。公开只读 GET `/api/runtime/tools/{id}/{version}` 供本机助手获取，未收录版本返回 404。
