# 二进制插件发布

插件作者在自己的 Rust、Kotlin 或 TypeScript 工具链中构建产物，再直接把 `.aio-plugin` 包上传到 aio-idea 插件中心。发布不依赖 GitHub Actions，不需要 Git 对象证明，也不要求把构建产物提交到 Git。一个功能仓库同时包含前端、后端和共享逻辑；子插件与父插件作为同一个包发布。

## 打包与上传

```bash
# 先执行本语言的构建和测试，确保清单引用的 artifact 已生成
aio plugin validate ./orders
aio plugin package ./orders --git https://example.com/team/orders.git --version 1.0.0 -o ./orders.aio-plugin

# 令牌通过本地环境或密钥管理器提供，不写入仓库
export AIO_PLUGIN_PUBLISH_TOKEN='<来源绑定的发布凭证>'
aio plugin publish ./orders.aio-plugin
```

`package` 默认输出 `<目录>/dist/plugin.aio-plugin`。Git 来源可以从当前插件目录自己的 `origin` 推导，没有 `.git` 时显式指定 `--git`；不继承父目录的 Git 信息。未提交的清单和被忽略的编译产物也可打包。`publish <目录> --version 1.0.0` 可以省去单独的打包命令；版本也可从该目录的 `Cargo.toml` 或 `package.json` 推导。

已经生成的包可以搬到任何目录或机器再发布。包内来源、版本和内容摘要不能通过命令参数改写。默认发布接口为 `https://aio.addzero.site/api/runtime/plugins/publish`，私有中心通过 `AIO_PLUGIN_PUBLISH_URL` 覆盖。CI 只是可选的构建与调用方式，不属于协议要求。

## v2 整包

清单声明 `schema_version = 2` 时，CLI 使用官方 `az-plugin-bundle::Bundle`，把后端、前端和声明的数据库迁移打入同一个包。process 后端必须是 Linux x86_64 ELF；Component 后端必须是对应 ABI 的 `.wasm`。源码提交字段 `commit` 是完整 Git SHA，独立目录使用零 SHA；正式内容身份是 `digest`。

v2 上传到 `/api/runtime/components/publish`，Content-Type 为 `application/vnd.aio.component+gzip`，不设置 Content-Encoding。`AIO_PLUGIN_PUBLISH_URL` 可指定宿主 origin 或完整发布接口；已有 `/plugins/publish` 地址会转换为同源 `/components/publish`。v2 包文件总量最多 64 MiB，文件最多 4096 个，压缩包和解压 JSON 各最多 96 MiB；全部路径和摘要经共享库验证。

宿主同步验证候选运行时并保存 PostgreSQL 组件发布记录，返回：

```json
{"data":{"source_id":"<来源UUID>","revision":"<整包digest>","state":"published"}}
```

CLI 校验返回的 digest 和 `published` 状态。发布使版本进入市场，租户安装与激活另行执行；上传成功不能作为租户界面可用的验收。v2 宿主已实现隔离前端挂载、会话调用桥、数据库和加密授权，实际部署仍须确认镜像、端点和数据库配置，并在安装后验收。

## 旧版包内容

以下 `az-plugin-package` 格式、`rev` 字段及后台激活任务用于未声明 `schema_version = 2` 的旧版清单，与 v2 整包协议分别处理。

包由共享库 `az-plugin-package` 编解码，当前格式为 2，是包含清单、后端 artifact 和可选前端编译资产的确定性 gzip JSON 容器。HTTP 直接发送包字节，类型为 `application/vnd.aio.plugin+gzip`，不设置 `Content-Encoding`。限制为清单 128 KiB、前后端合计 32 MiB、前端最多 256 个文件、压缩包与解压 JSON 各 48 MiB；多 gzip 成员、尾随数据、无效路径、缺失市场声明和摘要篡改均会被拒绝。格式 1 的未发布包需要重新打包。

- `version` 是发布者的 SemVer 版本号，同一 Git 来源不能用同一版本号覆盖不同内容。
- `rev` 是来源、版本、清单、可选源码参考和 artifact 摘要共同确定的完整 SHA-256，正式锁定实际运行包。
- `artifact_sha256` 校验二进制 artifact 字节。
- `frontend` 按相对路径保存各文件的 `content_base64` 与 `sha256`，整个集合也参与 `rev`；只修改前端同样必须发布新版本。
- `source_revision` 是可选完整 Git SHA，只作源码参考，不证明构建可复现，也不用于从 Git 下载包。

市场元数据必须在 `aio-plugin.toml` 中声明：

```toml
[plugin.marketplace]
title = "订单中心"
summary = "按租户隔离的订单页面与服务。"
license = "MIT"
tags = ["orders"]
```

当前在线目标为 `page-definition`、`wasm-component` 和 `process`。Rust 源码 `client/server` 声明不是可运行二进制包，必须使用源码装配流程；不能把尚未编译的 Dioxus crate 宣称为已交付在线插件。

前端联合包的文件契约见 [frontend-bundle.md](frontend-bundle.md)。v2 挂载与调用桥已有实现；Dioxus Web/Desktop 的具体制品仍须单独构建、安装并验收，不能从打包成功推断界面可用。

## 发布凭证

平台需要用 `AIO_PLUGIN_PUBLISH_ACCOUNTS` 明确授权可管理发布来源的账号。该账号还需具有当前租户的 `plugin:manage` 权限；普通租户用户不能任意认领别人的全局发布来源。

```bash
curl --fail --cookie 'aio_session=<登录会话>' \
  --header 'content-type: application/json' \
  --data '{"git":"https://example.com/team/orders.git"}' \
  https://aio.addzero.site/api/runtime/publish-credentials
```

响应中的 `token` 只返回一次。凭证固定到创建时的租户和 Git 来源，不能改发其他来源或租户；重复创建会轮换令牌。撤销接口：

```bash
curl --fail --request DELETE --cookie 'aio_session=<登录会话>' \
  https://aio.addzero.site/api/runtime/publish-credentials/<credential-id>
```

## 旧版保存与激活

宿主先检查凭证、包完整性、清单与能力边界，再将完整包和元数据保存 PostgreSQL `plugin_packages`，返回后台 `job_id`。工作队列完成 PageDefinition 校验、Component ABI 或 process 健康检查，最后通过现有生命周期事务激活。上传不是立即执行任意代码；安装器不会执行 Cargo、Kotlin、pnpm 或仓库脚本。

```json
{"data":{"job_id":"...","state":"queued","revision":"<包内容 SHA-256>","detail":"..."}}
```

CLI 会等待 `active` 或 `failed`。发布失败、启动失败或数据库切换失败保留上一活动版本；只有成功激活的版本才进入可下载目录和市场。重传相同内容可重试发布任务，但同版本不同内容必须更换 SemVer。

```bash
curl --fail --header 'authorization: Bearer <发布令牌>' \
  https://aio.addzero.site/api/runtime/publish-jobs/<job-id>

curl --fail --cookie 'aio_session=<登录会话>' \
  --output orders.aio-plugin \
  https://aio.addzero.site/api/runtime/packages/<包内容 SHA-256>
```

市场元数据和安装读取数据库。已发布包的安装、启用、回滚和重启恢复可从数据库重建丢失的本地缓存，不回退到远程 Git；Git 拉取只服务于显式源码或未发布仓库安装，并且仍锁定完整 Git 提交。

宿主限制并发发布和缓存资源；二进制中心默认最多 1024 个版本、2 GiB 包存储，可通过 `AIO_PLUGIN_PACKAGE_MAX_REVISIONS`、`AIO_PLUGIN_PACKAGE_MAX_BYTES` 调整。当前公网权限档不开放额外网络、文件系统或数据库能力。进程继续采用固定 digest 镜像、只读挂载、非 root 用户和资源限额，Wasm 实例按租户和版本隔离。
