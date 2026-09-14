# AIO CLI

AIO 的应用和社区插件命令行工具。npm 包只包含平台选择器；实际命令由仓库中的 Rust `az-aio-cli` 编译而来。

```bash
npm install --global @zjarlin/aio
aio --help
```

不安装也可以直接运行：

```bash
npx @zjarlin/aio --help
```

支持 macOS arm64/x64、Linux arm64/x64 和 Windows x64。Linux 包使用静态 musl 二进制。

本项目以 MIT 或 Apache-2.0 双重许可发布，完整文本见 `LICENSE-MIT` 和 `LICENSE-APACHE`。
