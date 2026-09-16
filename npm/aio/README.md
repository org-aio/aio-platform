# AIO CLI

AIO 的应用和社区插件命令行工具。npm 包只包含平台选择器；实际命令由仓库中的 Rust `az-aio-cli` 编译而来。

```bash
npm install --global @zjarlin/aio
aio --help
```

不安装也可以直接运行：

```bash
npx @zjarlin/aio --help
```

支持 macOS arm64/x64、Linux arm64/x64 和 Windows x64。Linux 包使用静态 musl 二进制。

本项目以 MIT 或 Apache-2.0 双重许可发布，完整文本见 `LICENSE-MIT` 和 `LICENSE-APACHE`。

## 从网页安装 CLI

此功能需要包含 `helper`/`tool` 命令的版本，以及同时升级的 AIO 市场服务。

```sh
npx -y @zjarlin/aio helper install
npx -y @zjarlin/aio tool list
```

注册一次本机助手后，在插件市场选择 CLI 并点击“安装到本机”。助手会在终端展示计划并等待确认。卸载使用 `npx -y @zjarlin/aio tool uninstall <id>`，先恢复工具配置，再移除包。

## CLI 随包技能

2026.9.18 起，`aio tool install <id> --version <version>` 自动将 npm 包内 `skills/<name>/` 安装到 `~/.agents/skills/<name>/`。`aio plugin init --kind cli` 默认生成使用技能；已有 CLI 使用 `--adopt` 接入时补齐缺失技能。卸载只清理 AIO 安装且内容未被用户修改的文件。
