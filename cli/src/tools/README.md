# 本机工具入口

仅解析 CLI 参数；安装模型、协议注册、生命周期和持久化在 az-tool 中共享。

`aio tool release setup|prepare|publish|sync` 集中实现 npm CLI 自动发布。setup 在作者电脑完成包所有权和可信工作流的一次性绑定；其余步骤只在 GitHub Actions 执行，使用同一个版本和源码标识，测试打包入口后发布公开 npm，最后通过 OIDC 同步市场。模型校验及安装描述位于共享 az-tool，服务器不执行项目代码。

发布和 Trusted Publisher 配置固定使用 npm 官方 registry，不继承本机镜像。npm OIDC 自动为公开仓库生成来源证明；私有源码仓库仍可发布公开 npm 包，不强制传入只支持公开源码的 `--provenance`。两者都保留打包入口、源码提交及完整性校验。
