use super::plugin_catalog::PluginCatalog;
use super::{
    RuntimeState, http_error::RuntimeError, request_context::authenticate_publish_manager,
};
use crate::runtime::{MarketplaceEntry, RuntimeResponse};
use axum::{Json, extract::State, http::HeaderMap};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
pub(super) struct RemoveRequest {
    git: String,
}

// 市场下架只记录来源，不删除发布包、租户安装或业务数据。
pub(super) async fn remove(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<RemoveRequest>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    authenticate_publish_manager(&state, &headers).await?;
    let mut exists = state.store()?.source_exists(&request.git).await?;
    if !exists
        && state
            .optional_service::<crate::runtime::server::components::Components>()?
            .is_some()
    {
        exists = state
            .components()?
            .catalog
            .source(&request.git)
            .await?
            .is_some();
    }
    if !exists {
        return Err(RuntimeError::not_found("插件市场条目不存在"));
    }
    state.store()?.mark_removed(&request.git).await?;
    Ok(Json(RuntimeResponse { data: () }))
}

// 下架只影响市场可安装目录；租户已安装的条目必须保留，否则会丢失停用和卸载入口。
pub(super) async fn retain_listed(
    store: &dyn PluginCatalog,
    entries: &mut Vec<MarketplaceEntry>,
) -> anyhow::Result<()> {
    let removed = store.removed_sources().await?;
    retain_installed_or_listed(&removed.into_iter().collect(), entries);
    Ok(())
}

fn retain_installed_or_listed(removed: &HashSet<String>, entries: &mut Vec<MarketplaceEntry>) {
    entries.retain(|entry| entry.installed || !removed.contains(&entry.git));
}

#[cfg(all(test, feature = "postgres"))]
#[path = "marketplace_removal_tests.rs"]
mod tests;

#[cfg(all(test, feature = "postgres"))]
mod unit_tests {
    use super::*;

    fn entry(git: &str, installed: bool) -> MarketplaceEntry {
        MarketplaceEntry {
            parent_git: None,
            parent_title: None,
            git: git.into(),
            rev: "rev".into(),
            title: git.into(),
            summary: String::new(),
            license: "MIT".into(),
            tags: Vec::new(),
            installed,
            menu_hidden: false,
            source_id: None,
            state: None,
            active_revision: None,
            runtime: None,
            capabilities: Default::default(),
        }
    }

    #[test]
    fn delisting_hides_only_uninstalled_entries() {
        let removed = HashSet::from(["https://example.com/removed.git".to_owned()]);
        let mut entries = vec![
            entry("https://example.com/removed.git", false),
            entry("https://example.com/removed.git", true),
            entry("https://example.com/listed.git", false),
        ];
        retain_installed_or_listed(&removed, &mut entries);
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|e| e.installed && e.git.ends_with("removed.git"))
        );
        assert!(entries.iter().any(|e| e.git.ends_with("listed.git")));
    }

    #[test]
    fn entries_without_removal_records_are_all_kept() {
        let mut entries = vec![entry("https://example.com/listed.git", false)];
        retain_installed_or_listed(&HashSet::new(), &mut entries);
        assert_eq!(entries.len(), 1);
    }
}
