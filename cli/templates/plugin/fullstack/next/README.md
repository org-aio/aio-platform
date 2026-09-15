# __TITLE__

next 全栈插件，包含前端计数、服务端计数接口和共享 TypeScript 模型。

```sh
pnpm install --frozen-lockfile
pnpm dev
```

独立开发时使用框架服务端 API；在 AIO 中通过宿主通信桥调用同一个接口。

```sh
pnpm build
pnpm typecheck
pnpm test
aio plugin dev .
```

`frontend/` 保存页面，`backend/` 保存业务与服务端定义，`shared/counter/` 保存类型及纯逻辑。
后端使用 Next.js App Router Route Handlers；`POST /api/counter` 接收 `{"value": 41}`，返回计数值 `42` 和宿主注入的租户。

AIO 内嵌入口使用构建时生成的静态页面，交互经通信桥调用真实框架服务端；不依赖宿主代理 SSR、Server Actions 或框架客户端路由。
构建将服务端依赖封装为 `dist/server.cjs`，浏览器资源写入 `dist/frontend/`；服务端归档只在启动时解包到独立临时目录，退出清理。
`aio plugin dev` 目前会在任一端变化时重建框架全栈产物。`aio-delivery.toml` 提供构建入口。

Next 服务端需要比基础 Node 示例更大的临时空间，清单声明 `temporary_storage_mb = 64`，需要使用 AIO 2026.9.15 或以上宿主。图片使用非优化模式，构建不携带 sharp 原生依赖。

框架说明：[Next.js 独立部署](https://nextjs.org/docs/app/api-reference/config/next-config-js/output)。
