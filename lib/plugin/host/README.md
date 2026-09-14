# 通用插件宿主

`configuration` 和 `identity` 定义装配边界；`runtime/model` 是 HTTP 模型，
`runtime/server` 承载安装、版本、实例、通信桥与交付，`runtime/client`、
`startup` 和 `Workspace` 承载浏览器生命周期。服务端与浏览器以 Cargo feature 分离。

产品负责提供身份实现、品牌、静态贡献、数据库和发布配置。此库不依赖业务插件，
不内置产品域名或仓库所有者。生产应用与本地开发宿主直接消费同一实现。
