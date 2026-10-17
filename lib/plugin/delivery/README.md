# 自动交付契约

构建清单、任务和文档快照的语言无关模型。清单仅用于作者构建，不进入插件运行时身份或业务请求。

文件入口调用 `parse` 读取 `aio-delivery.toml`；原生发现插件已持有 `BuildRecipe` 时调用 `validate_recipe`，复用参数数量、长度和空字符限制。来源切换、代数及原子入队属于宿主的 `DeliverySources` 契约。
