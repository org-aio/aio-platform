---
name: __NAME__
description: "使用 __NAME__ CLI 完成工具明确支持的命令或排查 CLI 使用问题时启用。"
---

# __NAME__

先执行 `__NAME__ --help` 核对当前版本的命令与参数，执行 `__NAME__ --version` 确认版本。未全局安装时可使用 `npx -y __NAME__`。仅处理用户要求的工具操作，以实际退出码和输出判断结果。

实现或接入业务能力后，同步维护本技能的触发描述、核心用法和必要约束；只记录当前 CLI 实际支持的命令。

通过 AIO 市场安装时，本文件随 npm 包自动分发到 `~/.agents/skills/__NAME__/SKILL.md`；直接 npm/npx 安装不会自动写入个人技能目录。
