# __TITLE__

同仓 Compose Web、Ktor 服务与共享 Kotlin 模型。执行 `sh scripts/build.sh` 构建并测试；推送默认分支后由插件中心自动发布。

## 自定义顶部分组与多级菜单

修改 `backend/service/resources/pages.json`，直接使用 `children` 树。根节点是宿主顶部分组，有 `children` 的子节点是目录，有 `body` 的节点是可点击页面。模板的“社区插件”可以替换，公共解析层自动生成内部 `scene` 和 `menu_path`。

例如把原页面放进“我的业务 → 运营中心 → 数据分析 → 数据大屏”，将整个文件改为：

```json
{
  "id": "my-business",
  "label": "我的业务",
  "children": [
    {
      "id": "business-operations",
      "label": "运营中心",
      "icon": "folder",
      "children": [
        {
          "id": "business-analysis",
          "label": "数据分析",
          "children": [
            {
              "id": "__NAME__",
              "label": "数据大屏",
              "body": { "kind": "frontend", "entry": "index.html" }
            }
          ]
        }
      ]
    }
  ]
}
```

生成后的本 README 已填入原页面 id；修改现有项目时保留自己的页面 id 和 body。多个页面放在同一 `children` 数组中；多个顶部分组写成场景根对象数组。根节点下直接放页面即可省去目录。目录最多八层、不能为空，且不能同时声明 `body`。权限 `required_permission` 只放在叶子页面，不从目录继承。

跨插件复用目录时，相同目录 id 的场景、父节点、标题和图标必须一致；页面 id 必须唯一，目录 id 不能与页面 id 冲突。

新增叶子页面还需把它的 id 加入 `aio-plugin.toml` 对应 `[[plugin.subplugins]]` 的 `pages` 数组；仅调整原页面的目录或标题无需修改该数组。

新增菜单不会自动生成 Compose 页面。多个菜单使用相同 `body.entry` 时会打开同一个前端入口；若需要不同页面，应实现对应前端入口并确保 HTML 被构建到 `dist/frontend`，再修改 `body.entry`。

```bash
sh scripts/build.sh
aio plugin validate .
```

构建后按项目发布流程发布新版本并激活，宿主才会采用新导航。修改本地 JSON 不会直接改变已安装版本。当前初始化命令没有 `--scene` 或 `--menu-path` 参数。

树状配置需要包含 `parse_page_definitions` 导航解析器的 CLI 与 AIO IDEA `2026.9.14` 或以上宿主，模板已声明最低宿主版本。此前发布的 npm `2026.5.10` 不含此功能，需要更新工具和宿主后再使用；升级宿主不会改写已有数据库页面列表。

完整双页面示例见 [中文插件开发指南：KMP 多级菜单](https://github.com/zjarlin/aio-platform/blob/main/docs/plugin/README.md#kmp-插件自定义顶部分组与多级菜单)。
