use super::super::{RuntimeState, plugin_catalog::PluginCatalog};
use crate::runtime::{MarketplaceEntry, RuntimeCatalog};
use anyhow::{Result, ensure};
#[cfg(feature = "postgres")]
use sqlx::PgPool;

/// 可覆盖的导航元素类别；三类标签都由租户部署方决定显示名。
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationLabelKind {
    Scene,
    Menu,
    Page,
}

impl NavigationLabelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Menu => "menu",
            Self::Page => "page",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "scene" => Some(Self::Scene),
            "menu" => Some(Self::Menu),
            "page" => Some(Self::Page),
            _ => None,
        }
    }
}

/// 一条导航标签覆盖；键是稳定标识，值只影响显示。
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct NavigationLabelOverride {
    pub kind: NavigationLabelKind,
    pub key: String,
    pub label: String,
}

/// 检查覆盖键、标签长度与字符集，避免把控制字符或空标签写进目录偏好。
pub(in crate::runtime::server) fn validate_navigation_label(
    kind: NavigationLabelKind,
    key: &str,
    label: &str,
) -> Result<()> {
    ensure!(
        !key.is_empty() && key.len() <= 512 && !key.chars().any(char::is_control),
        "导航覆盖键无效"
    );
    ensure!(
        !label.trim().is_empty() && label.len() <= 128 && !label.chars().any(char::is_control),
        "导航标签必须在 1 到 128 个字符之间"
    );
    ensure!(
        kind != NavigationLabelKind::Scene || !key.starts_with("component:"),
        "场景键必须是稳定场景标识"
    );
    Ok(())
}

#[cfg(feature = "postgres")]
pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::Executor::execute(pool, sqlx::raw_sql("CREATE TABLE IF NOT EXISTS tenant_hidden_plugin_menus (tenant_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(tenant_id,source_id)); CREATE TABLE IF NOT EXISTS tenant_navigation_labels (tenant_id TEXT NOT NULL, kind TEXT NOT NULL, target_key TEXT NOT NULL, label TEXT NOT NULL, PRIMARY KEY(tenant_id,kind,target_key))")).await?;
    Ok(())
}

pub(in crate::runtime::server) async fn enrich_entries(
    store: &dyn PluginCatalog,
    tenant: &str,
    entries: &mut [MarketplaceEntry],
) -> Result<()> {
    let hidden = store.hidden_sources(tenant).await?;
    for entry in entries {
        entry.menu_hidden = entry
            .source_id
            .as_ref()
            .is_some_and(|source| hidden.contains(source));
    }
    Ok(())
}

pub(in crate::runtime::server) async fn apply(
    state: &RuntimeState,
    tenant: &str,
    catalog: &mut RuntimeCatalog,
) -> Result<()> {
    let hidden = state.store()?.hidden_sources(tenant).await?;
    if hidden.is_empty() {
        return Ok(());
    }
    // 只提供导航偏好，完整页面目录继续用于挂载与服务授权。
    for source in &hidden {
        let prefix = format!("component:{source}:");
        catalog.hidden_pages.extend(
            catalog
                .pages
                .iter()
                .filter(|page| page.id.starts_with(&prefix))
                .map(|page| page.id.clone()),
        );
    }
    catalog
        .hidden_pages
        .extend(state.store()?.hidden_page_ids(tenant, &hidden).await?);
    catalog.hidden_pages.sort();
    catalog.hidden_pages.dedup();
    let labels = state.store()?.navigation_labels(tenant).await?;
    apply_labels(catalog, &labels);
    catalog.navigation_labels = labels
        .iter()
        .map(|entry| crate::runtime::NavigationLabelView {
            kind: entry.kind.as_str().into(),
            key: entry.key.clone(),
            label: entry.label.clone(),
        })
        .collect();
    Ok(())
}

/// 标签覆盖只改写显示名；页面标识、权限、挂载地址和菜单层级保持原样。
fn apply_labels(catalog: &mut RuntimeCatalog, labels: &[NavigationLabelOverride]) {
    for entry in labels {
        match entry.kind {
            NavigationLabelKind::Scene => {
                for page in catalog.pages.iter_mut() {
                    if page.scene.id == entry.key {
                        page.scene.label = entry.label.clone();
                    }
                }
            }
            NavigationLabelKind::Page => {
                for page in catalog.pages.iter_mut() {
                    if page.id == entry.key {
                        page.label = entry.label.clone();
                    }
                }
                for item in catalog.account_items.iter_mut() {
                    if item.id == entry.key {
                        item.label = entry.label.clone();
                    }
                }
                for item in catalog.plugin_settings.iter_mut() {
                    if item.page_id == entry.key {
                        item.label = entry.label.clone();
                    }
                }
            }
            NavigationLabelKind::Menu => {
                for page in catalog.pages.iter_mut() {
                    for group in page.menu_path.iter_mut() {
                        if group.id == entry.key {
                            group.label = entry.label.clone();
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{PageBody, PageDefinition, RuntimeCatalog, TenantView, UserView};
    use az_plugin_manifest::{MenuGroupDefinition, SceneDefinition};

    fn catalog() -> RuntimeCatalog {
        RuntimeCatalog {
            session_context: "session".into(),
            context: "context".into(),
            page_versions: Default::default(),
            tenant: TenantView {
                id: "tenant".into(),
                label: "租户".into(),
            },
            user: UserView {
                label: "用户".into(),
                handle: "@user".into(),
                initials: "用".into(),
            },
            pages: vec![PageDefinition {
                id: "component:source:users".into(),
                label: "用户管理".into(),
                icon: None,
                scene: SceneDefinition {
                    id: "system".into(),
                    label: "系统".into(),
                },
                menu_path: vec![MenuGroupDefinition {
                    id: "component-source-0-系统管理".into(),
                    label: "系统管理".into(),
                    icon: None,
                }],
                required_permission: None,
                body: PageBody::Frontend {
                    entry: "index.html".into(),
                },
            }],
            hidden_pages: Vec::new(),
            navigation_labels: Vec::new(),
            plugin_settings: Vec::new(),
            account_items: Vec::new(),
            plugins: Vec::new(),
        }
    }

    #[test]
    fn overrides_only_rewrite_display_names() -> Result<()> {
        let mut value = catalog();
        let before = value.pages[0].clone();
        apply_labels(
            &mut value,
            &[
                NavigationLabelOverride {
                    kind: NavigationLabelKind::Scene,
                    key: "system".into(),
                    label: "系统设置".into(),
                },
                NavigationLabelOverride {
                    kind: NavigationLabelKind::Menu,
                    key: "component-source-0-系统管理".into(),
                    label: "权限中心".into(),
                },
                NavigationLabelOverride {
                    kind: NavigationLabelKind::Page,
                    key: "component:source:users".into(),
                    label: "成员".into(),
                },
            ],
        );
        let page = &value.pages[0];
        assert_eq!(page.scene.label, "系统设置");
        assert_eq!(page.menu_path[0].label, "权限中心");
        assert_eq!(page.label, "成员");
        // 覆盖绝不改写标识、挂载入口、权限或层级。
        assert_eq!(page.id, before.id);
        assert_eq!(page.scene.id, before.scene.id);
        assert_eq!(page.menu_path[0].id, before.menu_path[0].id);
        assert_eq!(page.required_permission, before.required_permission);
        assert_eq!(page.body, before.body);
        Ok(())
    }

    #[test]
    fn rejects_blank_oversized_or_control_labels() {
        assert!(
            validate_navigation_label(NavigationLabelKind::Scene, "system", "系统设置").is_ok()
        );
        assert!(validate_navigation_label(NavigationLabelKind::Scene, "system", "   ").is_err());
        assert!(validate_navigation_label(NavigationLabelKind::Scene, "system", "").is_err());
        assert!(
            validate_navigation_label(NavigationLabelKind::Scene, "system", &"长".repeat(129))
                .is_err()
        );
        assert!(
            validate_navigation_label(NavigationLabelKind::Scene, "system", "带\u{0}控制符")
                .is_err()
        );
        assert!(validate_navigation_label(NavigationLabelKind::Scene, "", "系统设置").is_err());
    }

    #[test]
    fn scene_keys_must_be_stable_identifiers() {
        assert!(
            validate_navigation_label(
                NavigationLabelKind::Scene,
                "component:source:users",
                "跨页面场景"
            )
            .is_err()
        );
        assert!(
            validate_navigation_label(NavigationLabelKind::Page, "component:source:users", "成员")
                .is_ok()
        );
    }
}
