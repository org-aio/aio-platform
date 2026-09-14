# __TITLE__

同仓 Compose Web、Ktor 服务与共享 Kotlin 模型。执行 `sh scripts/build.sh` 构建并测试；推送默认分支后由插件中心自动发布。

## 自定义顶部分组与多级菜单

修改 `backend/service/resources/pages.json`。`scene` 决定宿主顶部的业务分组，模板的“社区插件”可以替换；`menu_path` 是从外到内的侧栏目录数组，每个元素是带 `id`、`label` 和可选 `icon` 的对象，不是字符串。页面本身的 `label` 是最终可点击的叶子菜单。

例如把原页面放进“我的业务 → 运营中心 → 数据分析 → 数据大屏”，保留原页面的 `id`、`body`，修改以下字段：

```json
{
  "scene": { "id": "my-business", "label": "我的业务" },
  "menu_path": [
    { "id": "business-operations", "label": "运营中心", "icon": "folder" },
    { "id": "business-analysis", "label": "数据分析", "icon": "folder" }
  ],
  "label": "数据大屏"
}
```

上面是字段片段，不能直接替换整个页面对象。一个插件可以在 `pages.json` 数组中声明多个页面：页面 `id` 必须不同；相同目录复用相同的路径对象，宿主会聚合为目录树。`menu_path: []` 表示直接显示在顶部分组下。相同 `scene.id` 必须使用相同标题；相同目录 `id` 的场景、父目录、标题和图标必须一致，且不能与页面 `id` 冲突。

新增菜单不会自动生成 Compose 页面。多个菜单使用相同 `body.entry` 时会打开同一个前端入口；若需要不同页面，应实现对应前端入口并确保 HTML 被构建到 `dist/frontend`，再修改 `body.entry`。

```bash
sh scripts/build.sh
aio plugin validate .
```

构建后按项目发布流程发布新版本并激活，宿主才会采用新导航。修改本地 JSON 不会直接改变已安装版本。当前初始化命令没有 `--scene` 或 `--menu-path` 参数。

完整双页面示例见 [中文插件开发指南：KMP 多级菜单](https://github.com/zjarlin/aio-platform/blob/main/docs/plugin/README.md#kmp-插件自定义顶部分组与多级菜单)。
