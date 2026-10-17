use super::{Components, model::Description};
use crate::identity::SessionContext;
use crate::runtime::{
    CapabilityManifest, InstalledPluginView, MarketplaceEntry, PageBody, PageDefinition,
    PluginRuntime, PluginState, RuntimeAccountItem, RuntimeCatalog,
};
use anyhow::{Context, Result};
use az_plugin_manifest::{MenuGroupDefinition, SceneDefinition};
use uuid::Uuid;

impl Components {
    pub async fn entries(&self, tenant: &str) -> Result<Vec<MarketplaceEntry>> {
        let rows = self.catalog.entries(tenant).await?;
        let mut entries = Vec::new();
        for row in rows {
            let metadata = row.version.metadata;
            let active_revision = row.installation.as_ref().map(|i| i.digest.clone());
            let source = row.source.id;
            let state = if let Some(installation) = &row.installation {
                Some(
                    self.installation_state(
                        source,
                        tenant,
                        &installation.digest,
                        installation.enabled,
                    )
                    .await,
                )
            } else {
                None
            };
            entries.push(MarketplaceEntry {
                git: row.source.git,
                rev: row.version.digest,
                title: metadata.title,
                summary: metadata.summary,
                license: metadata.license,
                tags: metadata.tags,
                menu_hidden: false,
                installed: active_revision.is_some(),
                source_id: Some(source.to_string()),
                state,
                active_revision,
                runtime: Some(if row.version.description.process {
                    PluginRuntime::Process
                } else {
                    PluginRuntime::WasmComponent
                }),
                capabilities: CapabilityManifest {
                    database: row.version.capabilities.database,
                    ..Default::default()
                },
                parent_git: metadata.parent,
                parent_title: metadata.parent_title,
            });
        }
        Ok(entries)
    }

    pub async fn append_catalog(
        &self,
        session: &SessionContext,
        catalog: &mut RuntimeCatalog,
    ) -> Result<()> {
        let tenant = &session.tenant_id;
        let rows = self.catalog.installations(Some(tenant), false).await?;
        for row in rows {
            let source = row.source.id;
            let installation = row.installation.context("安装记录缺失")?;
            let digest = installation.digest;
            let state = self
                .installation_state(source, tenant, &digest, installation.enabled)
                .await;
            let accessible = state == PluginState::Active
                && self
                    .host_permissions(
                        tenant,
                        &session.user_id,
                        &session.session_id,
                        &row.version.required_permissions,
                    )
                    .await?;
            append(
                catalog,
                source,
                row.menu_source,
                row.source.git,
                digest,
                installation.generation.to_string(),
                state,
                accessible,
                row.version.metadata.title,
                row.version.description,
                row.version.capabilities.database,
            );
        }
        if tenant == "development" {
            // 释放开发装配锁后再查询外部授权提供者。
            let development = self.development.read().await.clone();
            for (source, instance) in development.iter() {
                let parent = instance
                    .bundle
                    .manifest()
                    .plugin
                    .marketplace
                    .as_ref()
                    .and_then(|m| m.parent.as_ref());
                let menu_source = development
                    .iter()
                    .find(|(_, candidate)| parent == Some(&candidate.repository))
                    .map(|(id, _)| *id)
                    .unwrap_or(*source);
                let accessible = self
                    .host_permissions(
                        tenant,
                        &session.user_id,
                        &session.session_id,
                        &instance.bundle.manifest().plugin.required_permissions,
                    )
                    .await?;
                append(
                    catalog,
                    *source,
                    menu_source,
                    instance.source.clone(),
                    instance.bundle.digest().into(),
                    instance.generation.clone(),
                    PluginState::Active,
                    accessible,
                    instance
                        .bundle
                        .manifest()
                        .plugin
                        .marketplace
                        .as_ref()
                        .map(|metadata| metadata.title.clone())
                        .unwrap_or_else(|| instance.description.label.clone()),
                    instance.description.clone(),
                    instance.bundle.manifest().plugin.capabilities.database,
                );
            }
        }
        Ok(())
    }
}

// 参数对应安装记录的独立字段，保持数据库与开发态共用同一目录映射。
#[allow(clippy::too_many_arguments)]
fn append(
    catalog: &mut RuntimeCatalog,
    source: Uuid,
    menu_source: Uuid,
    git: String,
    digest: String,
    generation: String,
    state: PluginState,
    accessible: bool,
    title: String,
    description: Description,
    database: bool,
) {
    catalog.plugins.push(InstalledPluginView {
        source_id: source.to_string(),
        git,
        revision: digest.clone(),
        runtime: if description.process {
            PluginRuntime::Process
        } else {
            PluginRuntime::WasmComponent
        },
        state,
        capabilities: CapabilityManifest {
            database,
            ..Default::default()
        },
    });
    if state != PluginState::Active || !accessible {
        return;
    }
    for p in description.pages {
        let id = format!("component:{source}:{}", p.id);
        let permission = p
            .permission
            .as_deref()
            .map(|p| super::services::permission(source, p));
        let (scene_id, scene_label) = p.scene.unwrap_or_else(|| ("account".into(), "账户".into()));
        catalog
            .page_versions
            .insert(id.clone(), format!("{digest}:{generation}"));
        if p.surface == "settings" {
            catalog
                .plugin_settings
                .push(crate::runtime::PluginSettingsPage {
                    source_id: source.to_string(),
                    label: title.clone(),
                    page_id: id.clone(),
                });
        } else if p.surface != "workspace" {
            catalog.account_items.push(RuntimeAccountItem {
                id: id.clone(),
                label: p.label.clone(),
                icon: None,
                page_id: id.clone(),
                required_permission: permission.clone(),
            });
        }
        catalog.pages.push(PageDefinition {
            id,
            label: p.label,
            icon: None,
            scene: SceneDefinition {
                id: scene_id,
                label: scene_label,
            },
            menu_path: p
                .menu_path
                .into_iter()
                .enumerate()
                .map(|(i, label)| MenuGroupDefinition {
                    id: format!("component-{menu_source}-{i}-{label}"),
                    label,
                    icon: None,
                })
                .collect(),
            required_permission: permission,
            body: PageBody::Frontend { entry: p.entry },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_source_uses_the_installed_marketplace_title() -> Result<()> {
        let mut catalog: RuntimeCatalog = serde_json::from_value(serde_json::json!({
            "session_context": "session", "context": "workspace", "page_versions": {},
            "tenant": {"id": "tenant", "label": "工作区"},
            "user": {"label": "用户", "handle": "user", "initials": "U"},
            "pages": [], "plugins": []
        }))?;
        let description = serde_json::from_value(serde_json::json!({
            "label": "Internal agent runtime",
            "pages": [{"id": "settings", "label": "配置", "entry": "settings.html",
                "scene": null, "menu_path": [], "permission": null, "surface": "settings"}]
        }))?;
        append(
            &mut catalog,
            Uuid::nil(),
            Uuid::nil(),
            "plugin.git".into(),
            "revision".into(),
            "generation".into(),
            PluginState::Active,
            true,
            "用户看到的插件标题".into(),
            description,
            false,
        );
        assert_eq!(catalog.plugin_settings.len(), 1);
        assert_eq!(catalog.plugin_settings[0].label, "用户看到的插件标题");
        assert_eq!(catalog.pages[0].label, "配置");
        Ok(())
    }
    #[test]
    fn child_plugins_share_the_parent_menu_without_sharing_page_identity() -> Result<()> {
        let mut catalog: RuntimeCatalog = serde_json::from_value(serde_json::json!({
            "session_context":"session","context":"workspace","page_versions":{},
            "tenant":{"id":"tenant","label":"工作区"},"user":{"label":"用户","handle":"user","initials":"U"},"pages":[],"plugins":[]
        }))?;
        let parent = Uuid::new_v4();
        for (source, id, label) in [
            (parent, "chat", "对话"),
            (Uuid::new_v4(), "memory", "记忆图谱"),
        ] {
            let description = serde_json::from_value(
                serde_json::json!({"label":label,"pages":[{"id":id,"label":label,"entry":"index.html","scene":["workspace","工作空间"],"menu_path":["智能体"],"permission":null,"surface":"workspace"}]}),
            )?;
            append(
                &mut catalog,
                source,
                parent,
                "plugin.git".into(),
                "revision".into(),
                "generation".into(),
                PluginState::Active,
                true,
                label.into(),
                description,
                false,
            );
        }
        assert_eq!(
            catalog.pages[0].menu_path[0].id,
            catalog.pages[1].menu_path[0].id
        );
        assert_ne!(catalog.pages[0].id, catalog.pages[1].id);
        Ok(())
    }
}
