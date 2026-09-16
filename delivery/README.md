# 构建工作进程

`aio-delivery` 从宿主领取持久化任务，在受限 Docker 容器中构建锁定的源码提交，随后在容器外打包并提交结果。发布令牌和宿主接口不传入构建容器。

必需环境变量：`AIO_DELIVERY_TOKEN`、四种 `AIO_BUILD_IMAGE_RUST/KOTLIN/TYPESCRIPT/FULLSTACK`（必须是固定 digest）。可选 `AIO_DELIVERY_URL`（默认 `http://127.0.0.1:3080`）、`AIO_DELIVERY_ROOT`（默认 `/opt/aio-delivery`）、`AIO_CLI`（默认 `aio`）。

## 部署

构建 DNS 可通过 `AIO_BUILD_DNS` 指定；部署网络中需要域名地址覆盖时，可配置 `AIO_BUILD_HOSTS=github.com=IP`，多个地址以逗号分隔。此设置仅属于构建服务环境，保留正常 HTTPS 证书校验，不写入插件源码；重新配置服务时保留。地址应由运维按实际连通性维护。下载设置低速超时，网络失败按任务持久化退避并保留产物，其他源码仍可领取构建。

网络需要代理时，`AIO_BUILD_HTTP_PROXY` 可指向构建容器能访问的无凭据 HTTP 代理，统一应用于源码获取、包管理器和 Java HTTP/HTTPS 依赖下载。代理地址不得包含用户名或密码；发布与数据库凭据仍只留在宿主。252 使用[独立网络出口](egress/README.md)，无需开发机在线；已有活动版本不受构建网络中断影响。

Rust 适配器使用固定 nightly 的 Cargo `shallow-deps` 功能，只拉取锁定依赖提交，避免下载共享仓库的全部历史；构建结果仍绑定完整源码 SHA。

先部署共享协议与宿主，再安装 `aio-delivery`、`aio` 到 `/opt/aio-delivery/bin`。`install-git.sh` 为旧系统编译独立 Git，依赖 gcc、make、libcurl/openssl/zlib/expat 开发包，不覆盖系统 Git。镜像构建使用 `docker build --network host --build-arg BASE_IMAGE=<固定摘要>`；源码任务仍在独立网络的受限容器内执行。

在服务器执行 `node configure.cjs <Rust image ID> <Kotlin image ID> <TypeScript image ID> <Fullstack image ID>`，为宿主和工作进程生成一次性共享凭据。GitHub 发现凭据通过标准输入交给 `configure-github.cjs`，仅保存到宿主环境，不传给源码容器。安装 `aio-delivery.service` 后重启宿主并启用构建服务。

`inspect.cjs` 只读取仓库、任务和租户升级状态；传入任务 ID 可查看该任务的完整错误。运维依赖可用 `npm --prefix /opt/aio-delivery/ops install pg@8.16.3 --save-exact --ignore-scripts` 安装。脚本读取服务器已有数据库配置，不输出凭据。

## v2 与混合构建

工作进程按 `aio-plugin.toml` 的 schema_version 选择包格式；v2 包在容器外由共享 Bundle 库打包，提交完整源码 SHA 并经过宿主实际运行验证。发布队列持久化候选归档，重启后继续；发布成功与任务完成在同一事务提交，较旧目标不能覆盖新目标。组件租户安装独立推进，失败保留原实例，停用/卸载/回滚与自动升级使用同一锁。

`fullstack` 环境组合固定的 Rust、Kotlin、Node 工具链；`images/fullstack.Dockerfile` 接受已有 `RUST_IMAGE`、`KOTLIN_IMAGE`。服务器使用本地 image ID 时通过 `DOCKER_BUILDKIT=0 docker build --network host --build-arg RUST_IMAGE=sha256:... --build-arg KOTLIN_IMAGE=sha256:... - < images/fullstack.Dockerfile` 构建，再把生成的 image ID 配入工作进程。包含 Zig 与 cargo-zigbuild，可构建适用于宿主 process 镜像的 glibc 2.17 Linux 服务。

私有源码依赖不进入公开仓库。维护者把锁定提交导出 Git bundle 后，通过 `seed-source.cjs` 导入该插件和该构建镜像专属的持久缓存。插件从 `$HOME/.cache/aio/sources/<Git 地址 SHA256>` 读取完整提交；更新依赖 SHA 或构建镜像时同步预置缓存。构建容器始终没有 GitHub 或发布凭据。
