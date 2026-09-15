# 从插件市场安装本机 CLI

CLI 与宿主插件共用一个市场，条目包含 `cli` 标签及结构化安装描述。点击 **安装到本机** 使用 `aio://install/<id>?version=<SemVer>` 唤起本机助手。普通插件即使带 `cli` 标签，也不会执行本机安装。

## 首次使用

安装包含本功能的 AIO CLI 后，在终端执行：

```sh
npx -y @zjarlin/aio helper install
```

macOS 注册用户目录中的 AIO Helper.app，Windows 注册 HKCU，Linux 注册 xdg desktop entry。助手保存独立二进制，npx 缓存清理不影响链接。后续更新 CLI 后重新执行此命令更新助手。Linux 桌面需要 xdg-mime 和可用终端；macOS 需要系统自带的 osacompile、codesign。

```sh
npx -y @zjarlin/aio tool install codex-model-sync --version 0.4.1
npx -y @zjarlin/aio tool list
npx -y @zjarlin/aio tool uninstall codex-model-sync
npx -y @zjarlin/aio helper uninstall
```

安装前会展示计划，必须在本机终端输入 `yes`。链接不能传入命令、来源地址或自动确认标记。浏览器可能询问是否打开 AIO Helper。

## Codex 模型同步与 Auto Router

市场内置登记 `codex-model-sync 0.4.1`，支持 macOS、Linux 和 Windows。安装依次执行 `npm install --global codex-model-sync@0.4.1` 和 `codex-model-sync setup`，默认仍是同步模型及后台更新；直接使用 `npx -y codex-model-sync` 的旧用法不变。

需要自动路由时，在 macOS 本机显式开启桌面桥接和 hooks，然后完全退出并重新打开 Codex：

```sh
codex-model-sync router setup
codex-model-sync router status
```

候选模型来自本机 Codex 配置对应的 `/v1/models`，不写死模型名称。路由先根据任务难度与模型能力梯队选池，再用成本和成功率辅助筛选；Git 操作、“跑起来看看”和项目技术栈 CLI 意图可优先走低成本任务路径。当前使用规则路由，不包含训练后的 RouterLLM 分类器。详细配置及实际模型选择的查看方法见 [项目 README](https://github.com/zjarlin/codex-model-sync#readme)。

## 用命令上架 CLI

平台发布者登录插件市场，点击 **添加 CLI**。只需填写安装命令，例如 `npx -y codex-model-sync@0.4.1 setup`；Git 仓库地址可选，不要求编写 JSON 或 AIO 插件包。

- 系统自动使用仓库名或命令中的工具名作为标题，补充默认备注和 `cli` 标签；在详情页可随时 **编辑标题和备注**。
- 默认识别当前电脑系统，也可选择 macOS、Windows、Linux 或多个系统。命令需与所选系统匹配：macOS/Linux 使用 Bash（启用 pipefail），Windows 使用 PowerShell。
- **保存并安装** 先登记条目，再唤起本机助手。取消勾选「保存后打开本机助手安装」即可只上架。
- Git 地址支持不含凭据的公网 HTTPS 仓库，自动读取默认分支根目录的 README/README.md/README.markdown 作为插件说明。说明按提交缓存，可点击 **刷新 README**；读取失败不阻止上架，界面显示原因及仓库链接。
- 标题、备注、检测命令和卸载命令均为可选高级项。不会从 README 生成并执行安装或卸载脚本。

HTTP `POST /api/runtime/tools/register` 接收共享 `Registration` 模型；`PATCH /api/runtime/tools/{id}/details` 编辑展示资料并重新读取文档。两者复用平台发布权限，普通工作区管理员不能修改全局市场。相同安装命令、适用系统、检测和卸载步骤重复提交返回已有条目，不覆盖资料；不同执行方案生成新条目。

助手通过公开只读 `GET https://aio.addzero.site/api/runtime/tools/<id>/<version>` 获取已登记的安装描述。链接不能携带原始命令；服务器只保存命令，不执行它。AIO 不镜像或重打包第三方 CLI，直接使用工具自身的分发方式。

## 导入完整安装描述

需要按平台编排多个步骤时，仍可按照 `tools/registry/codex-model-sync-0.4.1.json` 创建 JSON。在宿主设置 `AIO_TOOL_REGISTRY_DIR` 指向目录，启动时验证并导入 PostgreSQL。市场显示每个工具最高 SemVer；同 ID、同版本的执行方案不会覆盖，变更步骤需递增版本。展示标题、备注和 Git 文档资料可独立编辑。

## 状态与恢复

安装步骤开始前保存原始描述；提供检测命令时，安装和检测都成功才记为 installed；没有检测命令时仅记为 executed，表示命令执行成功。安装失败保留记录。卸载使用保存的版本；没有卸载步骤时拒绝自动卸载并保留记录，有配置恢复需求的工具应将恢复步骤放在包删除之前。卸载失败保留进度，重试从下一未完成步骤继续。各工具通过当前用户目录中的锁串行操作。

网页无法直接读取本机安装情况，因此 CLI 不计入宿主的“已安装”。查看本机结果使用 `tool list`，不会把点击链接当作安装成功。首次发布需要同时交付宿主、市场前端和新版 npm CLI，旧 npm 版本不包含 helper/tool 命令。

## 边界

这一版提供安装、失败重试、原版本卸载和协议注册。更换版本前必须先卸载原版本，防止覆盖原始恢复描述；依赖缺失时提供修复说明，不自动安装整套包管理器。安装步骤退出成功只能代表条目声明的检测通过，第三方工具的配置恢复语义由其卸载命令负责。
