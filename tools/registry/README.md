# 工具安装描述

文件名使用 `<id>-<version>.json`。命令以 program 和 args 保存，不拼接 shell。requirements 检查安装依赖；detect 必须返回成功才记为已安装；uninstall 按顺序先恢复工具配置，再移除包。平台名为 macos、linux、windows。

宿主启动时导入 `codex-model-sync-0.4.1.json`，市场展示同 ID 的最高 SemVer。旧版本文件保留原始安装方案，不随新版本修改。

Codex 模型同步与 Auto Router 的安装固定使用 npm 0.4.1 并执行 `setup`，默认仅同步模型及注册后台更新任务。需要 Auto Router 时另行执行 `codex-model-sync router setup`，当前桌面桥接仅支持 macOS，完成后退出并重新打开 Codex。
