# 已发布服务包的本地容器运行

CLI 的容器适配器在 Linux x64 和 macOS Apple Silicon 使用同一个已验证的 Linux 服务包。Node 传输容器只负责标准输入输出与 Unix socket 转换；业务容器保留已有进程协议，不引入插件 ABI。数据库和通信桥连接由本地宿主提供，两个容器都禁用网络，不挂载 Docker socket。临时卷与本次启动的容器在退出时清理，正式开发数据库保留。

工具要求：本地 Docker 引擎、Node 22 或更高版本。传输镜像由 CLI 固定摘要；首次准备后可离线复用。Apple Silicon 的 x64 服务需要 Docker 引擎提供 amd64 模拟。
