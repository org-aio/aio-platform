use anyhow::{Context as _, Result, ensure};

use super::{MenuNode, PageDocument, SceneTree};
use crate::{MenuGroupDefinition, PageDefinition, SceneDefinition, validate_page_definitions};

const MAX_MENU_DEPTH: usize = 8;

/// 编写格式只在入站边界展开；存储和运行时继续消费经过校验的页面列表。
pub fn parse_page_definitions(bytes: &[u8]) -> Result<Vec<PageDefinition>> {
    let document: PageDocument = serde_json::from_slice(bytes).context(
        "页面定义格式无效：需要场景树、场景树数组或页面列表；目录只能包含 children，叶子必须包含 body",
    )?;
    let pages = match document {
        PageDocument::Pages(pages) => pages,
        PageDocument::Scene(scene) => expand_scenes(vec![scene])?,
        PageDocument::Scenes(scenes) => expand_scenes(scenes)?,
    };
    validate_page_definitions(&pages)?;
    Ok(pages)
}

fn expand_scenes(scenes: Vec<SceneTree>) -> Result<Vec<PageDefinition>> {
    let mut pages = Vec::new();
    for tree in scenes {
        ensure!(
            !tree.children.is_empty(),
            "场景 {} 的 children 不能为空",
            tree.id
        );
        let scene = SceneDefinition {
            id: tree.id,
            label: tree.label,
        };
        expand(tree.children, &scene, &mut Vec::new(), &mut pages)?;
    }
    Ok(pages)
}

fn expand(
    nodes: Vec<MenuNode>,
    scene: &SceneDefinition,
    path: &mut Vec<MenuGroupDefinition>,
    pages: &mut Vec<PageDefinition>,
) -> Result<()> {
    for node in nodes {
        match node {
            MenuNode::Group(group) => {
                ensure!(
                    path.len() < MAX_MENU_DEPTH,
                    "菜单 {} 超过 {MAX_MENU_DEPTH} 层目录",
                    group.id
                );
                ensure!(
                    !group.children.is_empty(),
                    "目录 {} 的 children 不能为空",
                    group.id
                );
                path.push(MenuGroupDefinition {
                    id: group.id,
                    label: group.label,
                    icon: group.icon,
                });
                expand(group.children, scene, path, pages)?;
                path.pop();
            }
            MenuNode::Page(page) => pages.push(PageDefinition {
                id: page.id,
                label: page.label,
                icon: page.icon,
                scene: scene.clone(),
                menu_path: path.clone(),
                required_permission: page.required_permission,
                body: page.body,
            }),
        }
    }
    Ok(())
}
