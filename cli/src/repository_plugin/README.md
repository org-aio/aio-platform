# 全栈插件装配与发布

以应用仓库的 `aio.toml` 为真源，负责 Git checkout、client/server 清单发现、Cargo feature 同步、两端 Dill 注册源码生成和卸载清理。`aio plugin validate [<仓库目录>]` 使用与运行时共享的无头清单契约，校验多语言 artifact、子插件依赖和 Component ABI；`aio plugin schema [<输出目录>]` 为其他语言输出同源 JSON Schema。

`aio plugin package <目录> --git <HTTPS Git> --version <SemVer> [-o <文件.aio-plugin>]` 校验预构建 artifact 并生成可搬运的二进制包，默认写入 `<目录>/dist/plugin.aio-plugin`。Git 来源也可从该目录自己的 `origin` 推导，不需要 Git 提交；未跟踪或被忽略的产物可以打包。源码提交只作为可选审计元数据，内容版本由整个包的 SHA-256 决定。

`aio plugin publish [<目录或文件.aio-plugin>] [--git <HTTPS Git>] [--version <SemVer>]` 通过来源绑定凭证直接上传包。AIO v2 整包使用 `/api/runtime/components/publish` 与 `application/vnd.aio.component+gzip`，宿主同步校验并发布；旧版二进制包继续使用 `/api/runtime/plugins/publish` 并轮询后台激活任务。发布目录时可从 `Cargo.toml` 或 `package.json` 推导版本；发布已有包时保留其来源和版本，不能用参数偷偷改写。默认插件中心为 `https://aio.addzero.site`，`AIO_PLUGIN_PUBLISH_URL` 可用完整发布地址或其他同源基址覆盖，`AIO_PLUGIN_PUBLISH_TOKEN` 提供凭证。不读取 GitHub Actions 环境变量，也不上传 Git 对象证明。
