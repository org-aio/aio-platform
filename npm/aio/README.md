# AIO CLI

AIO 的统一命令行入口。应用和社区插件命令由 Rust `az-aio-cli` 执行；`aio device` 从随包的 `@zjarlin/aio-device` 启动设备组件，不需要第二个全局 CLI。运行需要 Node.js 22.14 或更新版本。

```bash
npm install --global @zjarlin/aio
aio --help
```

不安装也可以直接运行：

```bash
npx @zjarlin/aio --help
```

设备连接、后台 worker、终端、桌面、文件和技能同步统一使用：

```sh
aio device --help
aio device connect
aio device worker --background
```

临时运行使用 `npx -y @zjarlin/aio device --help`；长期 worker 建议全局安装，以保留组件及原生依赖的固定位置。

`2026.10.11` 随包设备组件更新到 `0.12.2`，支持 Codex 网页中的文件选择、文件拖入、文件粘贴和目录拖入。配套宿主读取用户实际拖入的目录，保留原版界面的不透明沙箱；目录及全部子文件完整传到设备后再向原版界面提供根路径。设备保持现有配对并显式启用 Codex 网页访问。连接库测试与沙箱测试不代替真实 Codex/Buddy 使用验收。

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

`npm release` 由 main 或版本标签推送触发，发布任务使用既有 `npm` 环境的 `NPM_TOKEN`，遵循该环境的审核规则；构建任务不读取发布凭据。

维护者修正 npm 元数据或发布工作流后，可手动运行 `npm release` 并指定 `artifacts_run_id` 复用同版本构建。允许变更仅限 `npm/aio/`、npm 发布工作流和独立设备发布工作流。工作流验证五个平台已构建成功、产物未过期，且源码、前端、依赖和子模块均未变化；否则必须重新构建。仅发布阶段失败的运行也可以复用已成功的构建。

独立设备组件需要通过现有发布身份发布时，使用 `Device npm publication` 工作流。先审核设备标签对应的源码和测试，生成包含 `aio.source` 的 `aio-device-<version>.tgz`，把它放入 Platform Release，再提交 `version`、完整源码 `revision`、保存安装包的 `release_tag` 和整包 `sha256`。工作流复用 `npm` 环境审核，校验摘要、包名、版本、CLI 描述与源码绑定后发布；同版本已存在时必须与来源和整包完整性一致。设备发布完成后再更新统一 CLI 的随包依赖。
