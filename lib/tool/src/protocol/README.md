# 系统协议注册

按用户注册 `aio://`：macOS 使用 AppleScript applet 生成一次性 command 文件并由系统在终端打开，不申请控制 Terminal 的自动化权限；Windows 注册 HKCU；Linux 使用带 Terminal=true 的 desktop entry。协议仅唤起安装确认，不直接执行远程命令。助手复制独立二进制，避免 npx 缓存清理导致入口失效。
