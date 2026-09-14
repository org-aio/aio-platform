use anyhow::Result;
use az_plugin_manifest::{PageBody, SceneDefinition, SubpluginManifest};

use super::*;

#[test]
fn derives_account_item_from_declared_runtime_page() -> Result<()> {
    let manifest = PluginManifest {
        subplugins: vec![SubpluginManifest {
            id: "profile".to_owned(),
            dependencies: Vec::new(),
            pages: vec!["profile".to_owned()],
            routes: Vec::new(),
            account_actions: vec!["profile".to_owned()],
        }],
        ..PluginManifest::default()
    };
    let pages = vec![PageDefinition {
        id: "profile".to_owned(),
        label: "个人资料".to_owned(),
        icon: Some("user".to_owned()),
        scene: SceneDefinition {
            id: "account".to_owned(),
            label: "账户".to_owned(),
        },
        menu_path: Vec::new(),
        required_permission: Some("profile:view".to_owned()),
        body: PageBody::Text {
            title: "个人资料".to_owned(),
            content: "内容".to_owned(),
        },
    }];

    let items = runtime_account_items("source", &manifest, &pages)?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "source:profile");
    assert_eq!(items[0].page_id, "profile");
    assert_eq!(
        items[0].required_permission.as_deref(),
        Some("profile:view")
    );
    Ok(())
}
