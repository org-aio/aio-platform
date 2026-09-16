# __TITLE__

通过 AIO 初始化的 TypeScript CLI。

```sh
npm ci
npm test
node dist/cli.mjs --name AIO
```

发布与插件市场配置见 [AIO 自动发布说明](AIO.md)。初始化完成后提交锁文件和所有源码，后续推送默认分支自动发布开发版；版本标签发布正式版。

`skills/__NAME__/SKILL.md` 是随包的 AI 使用指南，实现 CLI 能力时同步维护。使用 AIO 2026.9.18+ 从市场安装后，技能自动进入 `~/.agents/skills/__NAME__/`，无需单独复制。其他安装方式不会隐式修改技能目录。
