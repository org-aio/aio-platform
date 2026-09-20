# npm 分发

`aio/` 是 `@zjarlin/aio` 的 JavaScript 入口包；平台二进制包只在发布工作流中由同一份配置生成，不提交构建产物。

GitHub 仓库可保留名为 `npm` 的 Environment 作为配置边界，并限制为 `main` 分支和 `v*` 标签；工作流不会绑定 Environment，因此不会暂停等待人工审批。首次发布前，在仓库或 Environment 中配置可创建 `@zjarlin` 公共包的 `NPM_TOKEN`；所有包创建后，再为 `.github/workflows/npm-release.yml` 配置 npm Trusted Publisher。

推送到 `main` 或推送 `v<CLI version>` 标签都会自动发布，不再要求人工审批。工作流先检查 npm 上是否已经存在当前版本；只有缺少包时才构建和发布，已全部存在时直接跳过。发布 tag 必须是 `v<CLI version>`，且所指提交必须属于 `origin/main`。工作流串行发布五个平台包，再发布入口包；失败后使用同一 tag 或再次推送 `main` 会跳过已经存在的版本。

也可从 `main` 手动触发发布，适用于首次发布或修正尚未发布的 npm 包配置，无需移动已有标签。手动发布同样校验 npm 与 Cargo 版本，并在确实缺少包时构建全部平台：

```bash
gh workflow run npm-release.yml --repo zjarlin/aio-platform --ref main
```

CLI 版本取自 `cli/Cargo.toml`。macOS arm64 与 Linux x64 包同时包含同版本 `aio-host` 及预编译 Web 资源，安装后本地开发无需克隆产品仓库；其他平台仅保持原有 CLI 能力。
