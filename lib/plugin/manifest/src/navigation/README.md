# 页面导航文档

开发者可用场景根节点和 `children` 编写导航树；接收端统一调用 `parse_page_definitions`，展开后用 `PageDefinition` 存储、鉴权和合并跨插件目录。模型与展开校验分离，不依赖 UI。输入也可直接使用协议页面列表，数据库中的页面仍使用列表，无需数据迁移。

场景根包含 `id`、`label`、`children`；目录包含 `id`、`label`、可选 `icon` 和 `children`；叶子包含 `id`、`label`、可选 `icon`、`required_permission` 和 `body`。目录不能同时携带页面体或权限；空场景、空目录、超过八层的目录、循环及冲突在提交前拒绝。
