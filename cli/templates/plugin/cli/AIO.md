# CLI 自动发布

本项目通过 AIO 初始化。`aio-cli.json` 声明市场名称、适用平台及安装后/卸载前参数；CLI 实现与 AIO 宿主无关。发布逻辑由固定版本的 AIO CLI 提供。

## 首次配置

先将项目推送到自己的 GitHub 仓库，再在项目目录执行一次：

```sh
npx -y @zjarlin/aio@__AIO_VERSION__ tool release setup
```

此命令从 Git origin 自动填写 npm 仓库地址，检查 CLI 声明；npm 包尚不存在时先测试并完成首次公开发布，然后配置该仓库 `aio-cli.yml` 的 Trusted Publisher。需要 npm 包写权限和账户二次验证；绑定成功后，日常推送通过短期 OIDC 身份发布，不需要每次验证或保存 npm token。也可在 npm 包设置页面完成同样的绑定。

AIO 市场校验 GitHub 签名身份，发布者须属于平台配置的 GitHub owner；其他作者先申请平台发布权限。首次设置填写了 `package.json` 的仓库地址后，请提交该改动。

## 日常开发

默认分支每次推送经过测试后，将 `package.json` 正式基础版本的下一个补丁版本生成为 `x.y.z-dev.<run>.g<sha>`，发布到 npm `next`，并更新同一个市场条目和 README。npm 元数据传播时会自动等待最多约十分钟。发布与同步可重试；失败保留旧市场版本。正式发布时更新 `package.json` 及锁文件版本并推送对应 `vX.Y.Z` 标签，工作流发布 npm `latest`。

市场显示已登记的最高 SemVer，安装命令固定精确版本。npm `latest` 不会被开发版改写。本机已安装版本不自动替换；升级前按市场说明卸载旧版本，再安装新版本。

CLI 必须支持 `--version`，返回打包版本。需要恢复配置时，将相应命令参数写入 `aio-cli.json` 的 `uninstall`，AIO 会先恢复配置，再卸载 npm 包。

## 随 CLI 分发技能

维护 `skills/__NAME__/SKILL.md`，目录名必须与 frontmatter 的 `name` 一致。npm 的 `files` 包含 `skills`。AIO 2026.9.18+ 通过 `aio tool install` 完成安装和检测后自动复制到 `~/.agents/skills`，随安装记录保存文件归属；卸载保留用户修改。直接 npm/npx 不执行写入技能目录的安装钩子。
