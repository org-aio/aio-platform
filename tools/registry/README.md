# 工具安装描述

文件名使用 `<id>-<version>.json`。命令以 program 和 args 保存，不拼接 shell。requirements 检查安装依赖；detect 必须返回成功才记为已安装；uninstall 按顺序先恢复工具配置，再移除包。平台名为 macos、linux、windows。
