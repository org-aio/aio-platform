# CLI 国内网络验收

验收日期：2026-09-14。实现提交：`d4582a2eaa22503689a2e206b42cf02057a72acf`。

## 结果

| 检查 | 结果 |
| --- | --- |
| CLI 单元测试 | 35 项通过 |
| CLI 集成测试 | 2 项通过，覆盖 10 种插件模板 × china/global，空 PATH、无效代理下离线生成 |
| 下载器故障测试 | 8 项通过，覆盖镜像回退、摘要拒绝、离线缓存、并发锁、死锁恢复、Range 续传和 JDK 版本检查 |
| 本机 JDK 选择 | 全局 JAVA_HOME 为 Azul 24，构建自动复用本机 OpenJDK 25，不改全局环境 |
| 国内镜像实际下载 | macOS arm64 的 OpenJDK 25.0.2、Node 26.5.1、pnpm 11.9.0 无代理下载并通过固定 SHA-256 校验 |
| 原始示例项目 | aio-plugin-kmp-demo 的 shared/service 测试、Wasm 前端编译、JVM 后端打包全部成功 |
| 新生成项目 | 使用锁定下载 JDK 和全新 KOTLIN_SHARED_CACHE_DIR，前后端完整构建成功 |
| 跨平台 CI | ubuntu-24.04、macos-15、windows-2025 的 CLI 测试、离线生成和工具链冷启动全部成功 |

CI 记录：[CLI network initialization #34798614439](https://github.com/zjarlin/aio-platform/actions/runs/34798614439)。CI 使用官方工具源检验平台兼容性；国内源可达性在本机未设置代理的环境验证，不代表所有运营商和企业网络。

## 复现

```sh
cargo test -p az-aio-cli --offline
node --test cli/tests/network.test.cjs
aio plugin init my-plugin --language kotlin
cd my-plugin
sh scripts/build.sh
```

可用 `AIO_JDK_DOWNLOAD=1` 选择锁定 JDK，用新的 `KOTLIN_SHARED_CACHE_DIR` 验证共享缓存冷启动。工具归档独立缓存且必须先通过摘要校验；pnpm 自己的依赖存储可能复用用户缓存。本次全新项目的日志中未出现 GitHub JDK、Node 或 pnpm 下载请求。

本机构建 CLI SHA-256：`7d2a1c892832c716e99de0ba588ff9108a049cdc8cf4407ef89e56630f8bdad2`。

## 发布与边界

本机 CLI 已替换，源码已推送 origin/main。原始 aio-plugin-kmp-demo 是未初始化 Git 的本地目录，构建入口已更新，未创建或推送仓库。

公共 npm 分发尚未成功。此前[发布任务 #34796108034](https://github.com/zjarlin/aio-platform/actions/runs/34796108034) 在创建 `@zjarlin/aio-darwin-arm64` 时收到 npm E404（包创建/访问权限不足）；需要修复 GitHub `npm` Environment 的 `NPM_TOKEN` 权限后发布，不能将本机更新视为 npm 已上架。

初始化不依赖网络；首次构建仍依赖可达的工具和依赖仓库，企业离线环境需要提前准备缓存或私有镜像。上游 pnpm 11.9.0 无 macOS x64 原生归档，该平台的全栈前端构建限制见生成项目 `NETWORK.md`。Rust Git 源依赖仍需能访问对应 Git 服务。
