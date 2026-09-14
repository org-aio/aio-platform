# CLI 网络验收

`node --test cli/tests/network.test.cjs` 验证真实下载器的回退、摘要拒绝、离线缓存和并发下载，以及本机 JDK 的版本校验。测试使用回环 HTTP 服务和临时目录，不访问公网、不修改全局环境。

`cargo test -p az-aio-cli` 验证参数解析和所有语言模板的离线初始化。
