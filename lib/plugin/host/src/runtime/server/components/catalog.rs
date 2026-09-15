use super::{Components, model::Description};
use crate::runtime::{
    CapabilityManifest, InstalledPluginView, MarketplaceEntry, PageBody, PageDefinition,
    PluginRuntime, PluginState, RuntimeAccountItem, RuntimeCatalog,
};
use anyhow::Result;
use az_plugin_manifest::{MenuGroupDefinition, SceneDefinition};
use sqlx::Row;
use uuid::Uuid;

impl Components {
    pub async fn entries(&self, tenant: &str) -> Result<Vec<MarketplaceEntry>> {
        let rows=sqlx::query("SELECT s.id,s.git,s.parent_git,p.digest,v.metadata,v.capabilities,v.description,i.digest AS installed_revision,i.enabled FROM component_sources s JOIN component_publications p ON p.source_id=s.id JOIN component_versions v ON v.digest=p.digest LEFT JOIN component_installations i ON i.source_id=s.id AND i.tenant_id=$1 ORDER BY v.metadata->>'title'").bind(tenant).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                let metadata: az_plugin_bundle::MarketplaceManifest =
                    serde_json::from_value(row.try_get("metadata")?)?;
                let active_revision: Option<String> = row.try_get("installed_revision")?;
                Ok(MarketplaceEntry {
                    git: row.try_get("git")?,
                    rev: row.try_get("digest")?,
                    title: metadata.title,
                    summary: metadata.summary,
                    license: metadata.license,
                    tags: metadata.tags,
                    installed: active_revision.is_some(),
                    source_id: Some(row.try_get::<Uuid, _>("id")?.to_string()),
                    state: active_revision.as_ref().map(|_| {
                        if row.get::<Option<bool>, _>("enabled") == Some(true) {
                            PluginState::Active
                        } else {
                            PluginState::Disabled
                        }
                    }),
                    active_revision,
                    runtime: Some(
                        if row.try_get::<serde_json::Value, _>("description")?["process"] == true {
                            PluginRuntime::Process
                        } else {
                            PluginRuntime::WasmComponent
                        },
                    ),
                    capabilities: CapabilityManifest {
                        database: row.try_get::<serde_json::Value, _>("capabilities")?["database"]
                            .as_bool()
                            .unwrap_or(false),
                        ..Default::default()
                    },
                    parent_git: metadata.parent,
                    parent_title: metadata.parent_title,
                })
            })
            .collect()
    }

    pub async fn append_catalog(&self, tenant: &str, catalog: &mut RuntimeCatalog) -> Result<()> {
        let rows=sqlx::query("SELECT s.id,s.git,i.digest,i.enabled,i.generation::TEXT,v.description,v.capabilities FROM component_sources s JOIN component_installations i ON i.source_id=s.id JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1").bind(tenant).fetch_all(&self.pool).await?;
        for row in rows {
            append(
                catalog,
                row.try_get("id")?,
                row.try_get("git")?,
                row.try_get("digest")?,
                row.try_get("generation")?,
                row.try_get("enabled")?,
                serde_json::from_value(row.try_get("description")?)?,
                row.try_get::<serde_json::Value, _>("capabilities")?["database"]
                    .as_bool()
                    .unwrap_or(false),
            );
        }
        if tenant == "development" {
            for (source, instance) in self.development.read().await.iter() {
                append(
                    catalog,
                    *source,
                    instance.source.clone(),
                    instance.bundle.digest().into(),
                    instance.generation.clone(),
                    true,
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
    git: String,
    digest: String,
    generation: String,
    enabled: bool,
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
        state: if enabled {
            PluginState::Active
        } else {
            PluginState::Disabled
        },
        capabilities: CapabilityManifest {
            database,
            ..Default::default()
        },
    });
    if !enabled {
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
        if p.surface != "workspace" {
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
                    id: format!("component-{source}-{i}-{label}"),
                    label,
                    icon: None,
                })
                .collect(),
            required_permission: permission,
            body: PageBody::Frontend { entry: p.entry },
        });
    }
}
