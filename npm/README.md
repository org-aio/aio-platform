# npm 分发

`aio/` 是 `@zjarlin/aio` 的 JavaScript 入口包；平台二进制包只在发布工作流中由同一份配置生成，不提交构建产物。

GitHub 仓库必须配置名为 `npm` 的受保护 Environment，并为它启用 required reviewers 和发布 tag 限制。首次发布前，在该 Environment 中配置可创建 `@zjarlin` 公共包的 `NPM_TOKEN`；所有包创建后，再为 `.github/workflows/npm-release.yml` 和 `npm` Environment 配置 npm Trusted Publisher。

发布 tag 必须是 `v<CLI version>`，且所指提交必须属于 `origin/main`。工作流串行发布五个平台包，再发布入口包；失败后使用同一 tag 重跑会跳过已经存在的版本。

也可从 `main` 手动触发发布，适用于首次发布或修正尚未发布的 npm 包配置，无需移动已有标签。Environment 的部署分支规则需允许 `main` 分支以及 `v*` 标签。手动发布同样校验 npm 与 Cargo 版本、构建全部平台并经过 `npm` Environment 审批：

```bash
gh workflow run npm-release.yml --repo zjarlin/aio-platform --ref main
```

CLI 版本取自 `cli/Cargo.toml`。macOS arm64 与 Linux x64 包同时包含同版本 `aio-host` 及预编译 Web 资源，安装后本地开发无需克隆产品仓库；其他平台仅保持原有 CLI 能力。
