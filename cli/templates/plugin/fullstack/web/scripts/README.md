# 构建与运行

`build.sh` 执行锁定依赖安装、框架构建、类型检查和真实接口测试。`archive.mjs` 将框架服务端文件及内部依赖链接封装为一个 `dist/server.cjs`；`runtime.cjs` 启动时解包到临时目录并加载框架原生入口。服务端文件不会进入公开的 `dist/frontend/`。
