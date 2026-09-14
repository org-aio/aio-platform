# __TITLE__

## 本地运行与调试

```sh
aio plugin dev .
aio plugin dev . --debug
```

无需提交或创建远端仓库。命令准备独立开发数据库、加载当前插件及必需依赖，并在保存源码后自动构建和重载；前端变化保留后端实例。状态保存在 `.aio/dev/`，退出保留数据。首次使用需要 Docker 与对应语言工具链；也可通过 `AIO_DEV_DATABASE_URL` 提供独立开发数据库。

完整配置、依赖覆盖和断点步骤见 [平台开发沙箱文档](https://github.com/zjarlin/aio-platform/blob/main/docs/development/README.md)。

同仓 Rust 前端、Component 后端与共享模型。执行 `zsh build.zsh` 构建并测试；推送默认分支后由插件中心自动发布。
