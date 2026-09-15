# 本机工具入口

仅解析 CLI 参数；安装模型、协议注册、生命周期和持久化在 az-tool 中共享。

`aio tool release setup|prepare|publish|sync` 集中实现 npm CLI 自动发布。setup 在作者电脑完成包所有权和可信工作流的一次性绑定；其余步骤只在 GitHub Actions 执行，使用同一个版本和源码标识，测试打包入口后发布公开 npm，最后通过 OIDC 同步市场。模型校验及安装描述位于共享 az-tool，服务器不执行项目代码。
