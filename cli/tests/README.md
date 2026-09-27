# CLI 网络验收

`node --test cli/tests/network.test.cjs` 验证真实下载器的回退、摘要拒绝、离线缓存和并发下载，以及本机 JDK 的版本校验。测试使用回环 HTTP 服务和临时目录，不访问公网、不修改全局环境。

`cargo test -p az-aio-cli` 验证参数解析和所有语言模板的离线初始化。

`cargo test -p az-aio-cli --test runtime_commands` 使用回环模拟服务验证连接、项目配置、代理路径前缀、动态帮助、完整参数转发、输出/非零退出码、环境覆盖、内置命令优先级、响应配额、AIO 登录和组件授权、0600 会话文件、UUID 配置校验及密钥脱敏，不访问公网。
