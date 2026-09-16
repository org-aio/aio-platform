# CLI 随包技能

`aio tool install` 完成安装和版本检测后，识别 AIO 标准 `npm install --global <package>@<version>` 步骤，从已安装包的 `skills/<name>/SKILL.md` 收集技能和附件，复制到 `~/.agents/skills/<name>/`。无需新增市场协议字段或执行包内脚本。模板和 CLI 作者应把 `skills` 加入 npm `files`。

每个技能必须包含名称与目录一致的 YAML `name` 和非空 `description`。不接受符号链接；每个包至多 128 个文件、总计 1 MiB，单个文件至多 256 KiB。安装记录保存文件哈希，拒绝覆盖其他来源或用户修改的内容；卸载保留被修改的文件。npm 自身安装钩子仍属于 npm 的原有行为，这个技能复制步骤不执行技能附件。

`Store::user()` 使用用户技能目录；`Store::new(root)` 使用隔离的 `root/skills`，适合测试或自定义安装仓库。命令字符串登记的任意第三方安装方式不推断包位置；此自动分发适用于 AIO 的结构化 npm CLI 安装计划。
