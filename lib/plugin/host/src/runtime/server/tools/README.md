# CLI 市场条目

同一市场返回宿主插件与 CLI 安装描述。CLI 由 `cli` 字段识别并带 `cli` 标签，不能送入宿主安装接口。本模块只保存和分发描述，不运行命令。

PostgreSQL 保存每个不可变版本。内置条目和 `AIO_TOOL_REGISTRY_DIR` 下的 JSON 在启动时导入；同 ID、同版本不会覆盖。公开只读 GET `/api/runtime/tools/{id}/{version}` 供本机助手获取，未收录版本返回 404。


平台发布者可通过 `POST /api/runtime/tools/register` 仅提交命令和可选仓库地址；服务端自动生成独立 ID、默认展示资料和对应系统的安装方案。`GET/PATCH /api/runtime/tools/{id}/details` 读取或编辑资料，README 使用只读 Git 对象获取，不检出工作区。`marketplace_tool_details` 保存可变展示资料和 README 快照，安装步骤仍不可变。刷新失败保留可见错误，入口权限与现有平台发布权限一致。

测试包含数据库与 HTTP 权限、重复登记、只改展示资料、未执行上架命令以及公网 Git README 读取。代理 Fake-IP 环境可在测试中用 `AIO_TEST_README_ADDRESS` 指定从公共 DNS 查询的真实 GitHub IP，生产地址校验不放宽。

`publication/` 接收 GitHub Actions 的 CLI 发布，验证短期签名身份及 npm 精确版本后更新市场。与手工登记共用不可变版本表，自动更新 README；不同仓库不能占用已有工具 ID。
