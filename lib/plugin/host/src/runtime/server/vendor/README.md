# 浏览器模块加载器

`es-module-shims.js` 来自 `es-module-shims@2.4.0` 的原始发行包，MIT 许可证见 `LICENSE`。
来源：https://github.com/guybedford/es-module-shims 。仅使用模块解析、import map 和 fetch hook，不使用 TypeScript 编译器，不依赖运行时 CDN。

2.4.0 的 `tsContentType` 正则包含一个意外空分支，会把所有 `text/javascript` 响应误判为
TypeScript，并尝试加载未打包的 `es-module-shims-typescript.js`。这里仅移除空分支；普通
JavaScript 继续走原生模块 shim，`application/typescript` 仍保留 MIME 识别。

npm tarball SHA-512：`+QaGYwkOpb5BjaL63YTis7eFYbcN1ZKUs6g3GR2qf4IuarUQNLlOgsSdw140Zycnc494/fixh547Kyny+ywYvQ==`。

沙箱 iframe 的不透明源无法可靠复用浏览器 HTTP 缓存。模块加载器在沙箱内执行，文件读取经宿主的摘要校验缓存；不放开 `allow-same-origin`，也不在宿主执行插件代码。
