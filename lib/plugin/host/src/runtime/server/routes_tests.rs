use az_plugin_manifest::{CapabilityManifest, PageActionDefinition, SceneDefinition};

use super::super::service_dispatch::ensure_route_allowed;
use super::*;
use crate::runtime::{InstalledPluginView, PluginRuntime, PluginState};

fn action_page(required_permission: Option<&str>) -> PageDefinition {
    PageDefinition {
        id: "counter".to_owned(),
        label: "Counter".to_owned(),
        icon: None,
        scene: SceneDefinition {
            id: "examples".to_owned(),
            label: "Examples".to_owned(),
        },
        menu_path: Vec::new(),
        required_permission: required_permission.map(str::to_owned),
        body: PageBody::Actions {
            title: "Counter".to_owned(),
            content: "0".to_owned(),
            state: [("count".to_owned(), serde_json::json!(0))]
                .into_iter()
                .collect(),
            actions: vec![PageActionDefinition {
                id: "increment".to_owned(),
                label: "+1".to_owned(),
            }],
        },
    }
}

#[test]
fn builds_manageable_entry_for_unlisted_plugin() {
    let entry = unlisted_entry(&InstalledPluginView {
        source_id: "source".to_owned(),
        git: "https://github.com/example/aio-plugin-kmp.git".to_owned(),
        revision: "0".repeat(40),
        runtime: PluginRuntime::PageDefinition,
        state: PluginState::Active,
        capabilities: Default::default(),
    });

    assert_eq!(entry.title, "aio-plugin-kmp");
    assert!(entry.installed);
    assert_eq!(entry.source_id.as_deref(), Some("source"));
    assert_eq!(entry.tags, vec!["unlisted"]);
}

#[test]
fn preserves_market_metadata_until_an_installed_manifest_overrides_it() {
    let mut entry = MarketplaceEntry {
        parent_git: None,
        parent_title: None,
        git: "https://github.com/example/plugin.git".to_owned(),
        rev: "1".repeat(40),
        title: "Plugin".to_owned(),
        summary: "Summary".to_owned(),
        license: "MIT".to_owned(),
        tags: vec!["example".to_owned()],
        installed: false,
        source_id: None,
        state: None,
        active_revision: None,
        runtime: Some(PluginRuntime::Process),
        capabilities: CapabilityManifest {
            network: vec!["api.example.com".to_owned()],
            filesystem: Vec::new(),
            database: false,
        },
    };

    assert_eq!(entry.runtime, Some(PluginRuntime::Process));
    assert_eq!(entry.capabilities.network, ["api.example.com"]);

    let git = entry.git.clone();
    enrich_marketplace_entry(
        &mut entry,
        &InstalledPluginView {
            source_id: "source".to_owned(),
            git,
            revision: "2".repeat(40),
            runtime: PluginRuntime::WasmComponent,
            state: PluginState::Active,
            capabilities: Default::default(),
        },
    );

    assert!(entry.installed);
    assert_eq!(entry.runtime, Some(PluginRuntime::WasmComponent));
    assert!(entry.capabilities.network.is_empty());
}

#[test]
fn allows_only_declared_route_prefixes() {
    let routes = vec!["echo".to_owned(), "jobs/status".to_owned()];
    assert!(ensure_route_allowed(&routes, "echo").is_ok());
    assert!(ensure_route_allowed(&routes, "echo/detail").is_ok());
    assert!(ensure_route_allowed(&routes, "jobs/status/current").is_ok());
    assert!(ensure_route_allowed(&routes, "jobs").is_err());
    assert!(ensure_route_allowed(&routes, "other").is_err());
}

#[test]
fn permission_gate_rejects_ungranted_account_contributions() {
    let permissions = vec!["workspace:view".to_owned()];

    assert!(permitted(None, &permissions));
    assert!(permitted(Some("workspace:view"), &permissions));
    assert!(!permitted(Some("plugin:manage"), &permissions));
}

#[test]
fn allows_only_declared_page_actions_with_permission() {
    let page = action_page(Some("counter:use"));
    let permissions = vec!["counter:use".to_owned()];

    assert!(ensure_page_action_allowed(&page, "increment", &permissions).is_ok());
    assert!(ensure_page_action_allowed(&page, "missing", &permissions).is_err());
    assert!(ensure_page_action_allowed(&page, "increment", &[]).is_err());
}

#[test]
fn action_result_cannot_change_published_actions() {
    let page = action_page(None);
    let valid = PageActionResult {
        body: PageBody::Actions {
            title: "Counter".to_owned(),
            content: "1".to_owned(),
            state: [("count".to_owned(), serde_json::json!(1))]
                .into_iter()
                .collect(),
            actions: vec![PageActionDefinition {
                id: "increment".to_owned(),
                label: "+1".to_owned(),
            }],
        },
    };
    assert!(validate_page_action_result(&page, &valid).is_ok());

    let changed = PageActionResult {
        body: PageBody::Actions {
            title: "Counter".to_owned(),
            content: "1".to_owned(),
            state: [("count".to_owned(), serde_json::json!(1))]
                .into_iter()
                .collect(),
            actions: vec![PageActionDefinition {
                id: "reset".to_owned(),
                label: "Reset".to_owned(),
            }],
        },
    };
    assert!(validate_page_action_result(&page, &changed).is_err());
    assert!(
        validate_page_action_result(
            &page,
            &PageActionResult {
                body: PageBody::Text {
                    title: "Counter".to_owned(),
                    content: "1".to_owned(),
                },
            },
        )
        .is_err()
    );
}
