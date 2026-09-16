use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::PluginManifest;
use serde_json::Value;
use sqlx::{PgPool, Row};

use super::{page_state, supervisor::ProcessInstance};
use crate::runtime::{
    InstalledPluginView, PageDefinition, PluginLifecycleEvent, PluginRuntime, PluginState,
    RuntimeAccountItem, RuntimeCatalog, TenantView, UserView,
};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_sources (
    id TEXT PRIMARY KEY,
    git TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS plugin_revisions (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES plugin_sources(id) ON DELETE CASCADE ON UPDATE CASCADE,
    revision TEXT NOT NULL,
    runtime TEXT NOT NULL,
    manifest JSONB NOT NULL,
    pages JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(source_id, revision)
);
CREATE TABLE IF NOT EXISTS plugin_runtime_instances (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    revision_id TEXT NOT NULL REFERENCES plugin_revisions(id),
    state TEXT NOT NULL,
    runtime_handle TEXT,
    endpoint TEXT,
    started_at TIMESTAMPTZ,
    stopped_at TIMESTAMPTZ
);
ALTER TABLE plugin_runtime_instances ADD COLUMN IF NOT EXISTS runtime_handle TEXT;
ALTER TABLE plugin_runtime_instances ADD COLUMN IF NOT EXISTS endpoint TEXT;
CREATE TABLE IF NOT EXISTS tenant_plugin_bindings (
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES plugin_sources(id) ON DELETE CASCADE ON UPDATE CASCADE,
    revision_id TEXT NOT NULL REFERENCES plugin_revisions(id),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id, source_id)
);
CREATE TABLE IF NOT EXISTS plugin_lifecycle_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    revision_id TEXT,
    lifecycle TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE plugin_revisions DROP CONSTRAINT IF EXISTS plugin_revisions_source_id_fkey;
ALTER TABLE plugin_revisions ADD CONSTRAINT plugin_revisions_source_id_fkey FOREIGN KEY (source_id) REFERENCES plugin_sources(id) ON DELETE CASCADE ON UPDATE CASCADE;
ALTER TABLE tenant_plugin_bindings DROP CONSTRAINT IF EXISTS tenant_plugin_bindings_source_id_fkey;
ALTER TABLE tenant_plugin_bindings ADD CONSTRAINT tenant_plugin_bindings_source_id_fkey FOREIGN KEY (source_id) REFERENCES plugin_sources(id) ON DELETE CASCADE ON UPDATE CASCADE;
"#;

pub struct PluginStore {
    pub(crate) pool: PgPool,
}

pub struct ProcessBinding {
    pub instance_id: String,
}

pub struct BoundRuntime {
    pub revision_id: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub artifact: String,
}

pub struct ServiceBinding {
    pub runtime: PluginRuntime,
    pub revision: String,
    pub endpoint: Option<String>,
    pub routes: Vec<String>,
}

pub struct PageBinding {
    pub source_id: String,
    pub activation_generation: String,
    pub revision_id: String,
    pub state_generation: Option<i64>,
    pub page: PageDefinition,
    pub service: ServiceBinding,
}

pub struct ProcessTarget {
    pub tenant_id: String,
    pub source_id: String,
    pub revision_id: String,
    pub revision: String,
}

pub struct WasmTarget {
    pub tenant_id: String,
    pub source_id: String,
    pub revision_id: String,
    pub revision: String,
    pub artifact: String,
}

impl PluginStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::raw_sql(SCHEMA)
            .execute(&self.pool)
            .await
            .context("创建插件运行时表失败")?;
        super::marketplace_store::migrate(&self.pool).await?;
        super::tools::migrate(&self.pool).await?;
        super::publisher_store::migrate(&self.pool).await?;
        super::package_store::migrate(&self.pool).await?;
        super::delivery::migrate(&self.pool).await?;
        page_state::migrate(&self.pool).await?;
        super::navigation::migrate(&self.pool).await?;
        sqlx::raw_sql(include_str!("../../generated/worker/schema.sql"))
            .execute(&self.pool)
            .await?;
        sqlx::raw_sql(include_str!("../../generated/personal_config/schema.sql"))
            .execute(&self.pool)
            .await?;
        self.migrate_published_source_ids().await?;
        Ok(())
    }

    pub async fn has_plugins(&self, tenant_id: &str) -> Result<bool> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM tenant_plugin_bindings WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    pub async fn source_id(&self, git: &str) -> Result<Option<String>> {
        sqlx::query_scalar("SELECT id FROM plugin_sources WHERE git = $1")
            .bind(git)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn record_lifecycle_event(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision_id: Option<&str>,
        lifecycle: &str,
        detail: &str,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            revision_id,
            lifecycle,
            detail,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn lifecycle_events(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Vec<PluginLifecycleEvent>> {
        let rows = sqlx::query(
            "SELECT id, lifecycle, detail, created_at::TEXT AS created_at FROM plugin_lifecycle_events WHERE tenant_id = $1 AND source_id = $2 ORDER BY created_at DESC LIMIT 20",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PluginLifecycleEvent {
                    id: row.try_get("id")?,
                    lifecycle: row.try_get("lifecycle")?,
                    detail: row.try_get("detail")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn catalog(
        &self,
        tenant_id: &str,
        tenant_label: &str,
        user: UserView,
    ) -> Result<RuntimeCatalog> {
        let rows = sqlx::query(
            "SELECT sources.id, sources.git, revisions.id AS revision_id, revisions.revision, revisions.runtime, revisions.manifest, revisions.pages, bindings.enabled, bindings.updated_at::TEXT AS activation_generation FROM tenant_plugin_bindings bindings JOIN plugin_sources sources ON sources.id = bindings.source_id JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.tenant_id = $1 ORDER BY sources.git",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        let mut pages = Vec::new();
        let mut page_versions = std::collections::BTreeMap::new();
        let mut account_items = Vec::new();
        let mut plugins = Vec::new();
        for row in rows {
            let source_id: String = row.try_get("id")?;
            let enabled: bool = row.try_get("enabled")?;
            let manifest = serde_json::from_value::<PluginManifest>(row.try_get("manifest")?)
                .context("解析已安装插件清单失败")?;
            let value: Value = row.try_get("pages")?;
            if enabled {
                let revision_id: String = row.try_get("revision_id")?;
                let mut plugin_pages = serde_json::from_value::<Vec<PageDefinition>>(value)?;
                self.overlay_page_states(tenant_id, &revision_id, &mut plugin_pages)
                    .await?;
                account_items.extend(runtime_account_items(&source_id, &manifest, &plugin_pages)?);
                let revision: String = row.try_get("revision")?;
                let generation: String = row.try_get("activation_generation")?;
                let version = serde_json::to_string(&(&source_id, revision, generation))?;
                for page in &plugin_pages {
                    page_versions.insert(page.id.clone(), version.clone());
                }
                pages.extend(plugin_pages);
            }
            plugins.push(InstalledPluginView {
                source_id,
                git: row.try_get("git")?,
                revision: row.try_get("revision")?,
                runtime: parse_runtime(row.try_get("runtime")?)?,
                state: if enabled {
                    PluginState::Active
                } else {
                    PluginState::Disabled
                },
                capabilities: manifest.capabilities,
            });
        }
        ensure_unique_pages(&pages)?;
        Ok(RuntimeCatalog {
            plugin_settings: Vec::new(),
            session_context: String::new(),
            context: String::new(),
            page_versions,
            tenant: TenantView {
                id: tenant_id.to_owned(),
                label: tenant_label.to_owned(),
            },
            user,
            pages,
            hidden_pages: Vec::new(),
            account_items,
            plugins,
        })
    }

    pub async fn set_enabled(
        &self,
        tenant_id: &str,
        source_id: &str,
        enabled: bool,
        instance: Option<&ProcessInstance>,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let revision_id = sqlx::query_scalar::<_, String>("UPDATE tenant_plugin_bindings SET enabled = $3, updated_at = now() WHERE tenant_id = $1 AND source_id = $2 RETURNING revision_id")
            .bind(tenant_id).bind(source_id).bind(enabled).fetch_optional(&mut *transaction).await?
            .context("插件绑定不存在")?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        if enabled {
            start_instance(&mut transaction, tenant_id, &revision_id, instance).await?;
        }
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            Some(&revision_id),
            if enabled { "activate" } else { "deactivate" },
            "运行时状态已切换",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn uninstall(&self, tenant_id: &str, source_id: &str) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        page_state::delete_source_states(&mut transaction, tenant_id, source_id).await?;
        let result = sqlx::query(
            "DELETE FROM tenant_plugin_bindings WHERE tenant_id = $1 AND source_id = $2",
        )
        .bind(tenant_id)
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
        ensure!(result.rows_affected() == 1, "插件绑定不存在");
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            None,
            "uninstall",
            "租户组合已移除插件",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn rollback_target(&self, tenant_id: &str, source_id: &str) -> Result<BoundRuntime> {
        let row = sqlx::query(
            "SELECT revisions.id, revisions.revision, revisions.runtime, revisions.manifest->'runtime'->>'artifact' AS artifact FROM plugin_revisions revisions JOIN tenant_plugin_bindings bindings ON bindings.source_id = revisions.source_id WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND revisions.id <> bindings.revision_id AND (length(revisions.revision)<>64 OR EXISTS(SELECT 1 FROM plugin_packages p WHERE p.revision=revisions.revision)) ORDER BY revisions.created_at DESC LIMIT 1",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await?
        .context("没有可回滚的历史版本")?;
        Ok(BoundRuntime {
            revision_id: row.try_get("id")?,
            revision: row.try_get("revision")?,
            runtime: parse_runtime(row.try_get("runtime")?)?,
            artifact: row.try_get("artifact")?,
        })
    }

    pub async fn rollback_to(
        &self,
        tenant_id: &str,
        source_id: &str,
        target: &BoundRuntime,
        instance: Option<&ProcessInstance>,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        super::delivery::exclude_revision(&mut transaction, tenant_id, source_id).await?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        let result = sqlx::query("UPDATE tenant_plugin_bindings SET revision_id = $3, enabled = TRUE, updated_at = now() WHERE tenant_id = $1 AND source_id = $2 AND EXISTS (SELECT 1 FROM plugin_revisions WHERE id = $3 AND source_id = $2)")
            .bind(tenant_id).bind(source_id).bind(&target.revision_id).execute(&mut *transaction).await?;
        ensure!(result.rows_affected() == 1, "回滚目标不属于当前插件");
        start_instance(&mut transaction, tenant_id, &target.revision_id, instance).await?;
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            Some(&target.revision_id),
            "rollback",
            "已切回上一健康版本",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn active_service(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<ServiceBinding>> {
        let row = sqlx::query(
            "SELECT revisions.revision, revisions.runtime, revisions.manifest, instances.endpoint FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id LEFT JOIN LATERAL (SELECT endpoint FROM plugin_runtime_instances WHERE tenant_id = bindings.tenant_id AND revision_id = revisions.id AND state = 'active' ORDER BY started_at DESC LIMIT 1) instances ON TRUE WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND bindings.enabled = TRUE AND revisions.runtime IN ('wasm-component', 'process')",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            let manifest = serde_json::from_value::<PluginManifest>(row.try_get("manifest")?)?;
            manifest
                .runtime
                .as_ref()
                .context("活动服务插件缺少 runtime 清单")?;
            Ok(ServiceBinding {
                runtime: parse_runtime(row.try_get("runtime")?)?,
                revision: row.try_get("revision")?,
                endpoint: row.try_get("endpoint")?,
                routes: manifest
                    .subplugins
                    .iter()
                    .flat_map(|plugin| plugin.routes.iter().cloned())
                    .collect(),
            })
        })
        .transpose()
    }

    pub async fn active_page_binding(
        &self,
        tenant_id: &str,
        page_id: &str,
    ) -> Result<Option<PageBinding>> {
        let rows = sqlx::query(
            "SELECT sources.id AS source_id, bindings.updated_at::TEXT AS activation_generation, revisions.id AS revision_id, revisions.revision, revisions.runtime, revisions.manifest, revisions.pages, instances.endpoint FROM tenant_plugin_bindings bindings JOIN plugin_sources sources ON sources.id = bindings.source_id JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id LEFT JOIN LATERAL (SELECT endpoint FROM plugin_runtime_instances WHERE tenant_id = bindings.tenant_id AND revision_id = revisions.id AND state = 'active' ORDER BY started_at DESC LIMIT 1) instances ON TRUE WHERE bindings.tenant_id = $1 AND bindings.enabled = TRUE ORDER BY sources.id",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        let mut binding = None;
        for row in rows {
            let revision_id: String = row.try_get("revision_id")?;
            let mut pages = serde_json::from_value::<Vec<PageDefinition>>(row.try_get("pages")?)?;
            let generations = self
                .overlay_page_states(tenant_id, &revision_id, &mut pages)
                .await?;
            let Some(page) = pages.into_iter().find(|page| page.id == page_id) else {
                continue;
            };
            ensure!(binding.is_none(), "活动插件页面 id 重复: {page_id}");
            let manifest = serde_json::from_value::<PluginManifest>(row.try_get("manifest")?)?;
            binding = Some(PageBinding {
                source_id: row.try_get("source_id")?,
                activation_generation: row.try_get("activation_generation")?,
                revision_id,
                state_generation: generations.get(page_id).copied(),
                page,
                service: ServiceBinding {
                    runtime: parse_runtime(row.try_get("runtime")?)?,
                    revision: row.try_get("revision")?,
                    endpoint: row.try_get("endpoint")?,
                    routes: manifest
                        .subplugins
                        .iter()
                        .flat_map(|plugin| plugin.routes.iter().cloned())
                        .collect(),
                },
            });
        }
        Ok(binding)
    }

    pub async fn bound_runtime(&self, tenant_id: &str, source_id: &str) -> Result<BoundRuntime> {
        let row = sqlx::query(
            "SELECT revisions.id, revisions.revision, revisions.runtime, revisions.manifest->'runtime'->>'artifact' AS artifact FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.tenant_id = $1 AND bindings.source_id = $2",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await?
        .context("插件绑定不存在")?;
        Ok(BoundRuntime {
            revision_id: row.try_get("id")?,
            revision: row.try_get("revision")?,
            runtime: parse_runtime(row.try_get("runtime")?)?,
            artifact: row.try_get("artifact")?,
        })
    }

    pub async fn active_wasm_revision(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<String>> {
        sqlx::query_scalar(
            "SELECT revisions.revision FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND bindings.enabled = TRUE AND revisions.runtime = 'wasm-component'",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn active_process(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<ProcessBinding>> {
        let row = sqlx::query(
            "SELECT instances.runtime_handle FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id JOIN plugin_runtime_instances instances ON instances.revision_id = revisions.id AND instances.tenant_id = bindings.tenant_id AND instances.state = 'active' WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND bindings.enabled = TRUE AND revisions.runtime = 'process' ORDER BY instances.started_at DESC LIMIT 1",
        )
        .bind(tenant_id)
        .bind(source_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ProcessBinding {
                instance_id: row
                    .try_get::<Option<String>, _>("runtime_handle")?
                    .context("process 插件实例缺少 runtime_handle")?,
            })
        })
        .transpose()
    }

    pub async fn enabled_process_targets(&self) -> Result<Vec<ProcessTarget>> {
        let rows = sqlx::query(
            "SELECT bindings.tenant_id, bindings.source_id, revisions.id, revisions.revision FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.enabled = TRUE AND revisions.runtime = 'process' ORDER BY bindings.tenant_id, bindings.source_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ProcessTarget {
                    tenant_id: row.try_get("tenant_id")?,
                    source_id: row.try_get("source_id")?,
                    revision_id: row.try_get("id")?,
                    revision: row.try_get("revision")?,
                })
            })
            .collect()
    }

    pub async fn stop_orphan_process_records(&self) -> Result<()> {
        sqlx::query(
            "UPDATE plugin_runtime_instances instances SET state = 'stopped', stopped_at = now() WHERE instances.state = 'active' AND instances.runtime_handle IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.tenant_id = instances.tenant_id AND bindings.revision_id = instances.revision_id AND bindings.enabled = TRUE AND revisions.runtime = 'process')",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn enabled_wasm_targets(&self) -> Result<Vec<WasmTarget>> {
        let rows = sqlx::query(
            "SELECT bindings.tenant_id, bindings.source_id, revisions.id, revisions.revision, revisions.manifest->'runtime'->>'artifact' AS artifact FROM tenant_plugin_bindings bindings JOIN plugin_revisions revisions ON revisions.id = bindings.revision_id WHERE bindings.enabled = TRUE AND revisions.runtime = 'wasm-component' ORDER BY bindings.tenant_id, bindings.source_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(WasmTarget {
                    tenant_id: row.try_get("tenant_id")?,
                    source_id: row.try_get("source_id")?,
                    revision_id: row.try_get("id")?,
                    revision: row.try_get("revision")?,
                    artifact: row.try_get("artifact")?,
                })
            })
            .collect()
    }

    pub async fn verify_revision_pages(
        &self,
        revision_id: &str,
        pages: &[PageDefinition],
    ) -> Result<()> {
        let stored =
            sqlx::query_scalar::<_, Value>("SELECT pages FROM plugin_revisions WHERE id = $1")
                .bind(revision_id)
                .fetch_optional(&self.pool)
                .await?
                .context("插件 revision 不存在")?;
        let stored = serde_json::from_value::<Vec<PageDefinition>>(stored)?;
        ensure!(stored == pages, "运行时插件重启后 PageDefinition 发生漂移");
        Ok(())
    }

    pub async fn recover_process_instance(
        &self,
        target: &ProcessTarget,
        instance: &ProcessInstance,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        stop_instances(&mut transaction, &target.tenant_id, &target.source_id).await?;
        start_instance(
            &mut transaction,
            &target.tenant_id,
            &target.revision_id,
            Some(instance),
        )
        .await?;
        record_event(
            &mut transaction,
            &target.tenant_id,
            &target.source_id,
            Some(&target.revision_id),
            "recover",
            "宿主启动时已恢复 process 插件实例",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn restore_process_instance(
        &self,
        tenant_id: &str,
        source_id: &str,
        target: &BoundRuntime,
        instance: &ProcessInstance,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        stop_instances(&mut transaction, tenant_id, source_id).await?;
        start_instance(
            &mut transaction,
            tenant_id,
            &target.revision_id,
            Some(instance),
        )
        .await?;
        record_event(
            &mut transaction,
            tenant_id,
            source_id,
            Some(&target.revision_id),
            "restore",
            "数据库切换失败后已恢复原 process 插件实例",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn recover_wasm_instance(&self, target: &WasmTarget) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        stop_instances(&mut transaction, &target.tenant_id, &target.source_id).await?;
        start_instance(
            &mut transaction,
            &target.tenant_id,
            &target.revision_id,
            None,
        )
        .await?;
        record_event(
            &mut transaction,
            &target.tenant_id,
            &target.source_id,
            Some(&target.revision_id),
            "recover",
            "宿主启动时已恢复租户 Wasm Component 实例",
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn ensure_unique_pages(pages: &[PageDefinition]) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    for page in pages {
        ensure!(!page.id.trim().is_empty(), "运行时页面 id 不能为空");
        ensure!(
            ids.insert(page.id.as_str()),
            "运行时页面 id 重复: {}",
            page.id
        );
    }
    Ok(())
}

fn runtime_account_items(
    source_id: &str,
    manifest: &PluginManifest,
    pages: &[PageDefinition],
) -> Result<Vec<RuntimeAccountItem>> {
    let mut items = Vec::new();
    for action in manifest
        .subplugins
        .iter()
        .flat_map(|subplugin| subplugin.account_actions.iter())
    {
        let page = pages
            .iter()
            .find(|page| page.id == *action)
            .with_context(|| format!("账户动作缺少已声明页面: {action}"))?;
        items.push(RuntimeAccountItem {
            id: format!("{source_id}:{action}"),
            label: page.label.clone(),
            icon: page.icon.clone(),
            page_id: page.id.clone(),
            required_permission: page.required_permission.clone(),
        });
    }
    Ok(items)
}

pub(super) async fn record_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    source_id: &str,
    revision_id: Option<&str>,
    lifecycle: &str,
    detail: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO plugin_lifecycle_events (id, tenant_id, source_id, revision_id, lifecycle, detail) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(uuid::Uuid::new_v4().to_string()).bind(tenant_id).bind(source_id)
        .bind(revision_id).bind(lifecycle).bind(detail).execute(&mut **transaction).await?;
    Ok(())
}

pub(super) async fn stop_instances(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    source_id: &str,
) -> Result<()> {
    sqlx::query("UPDATE plugin_runtime_instances SET state = 'stopped', stopped_at = now() WHERE tenant_id = $1 AND state = 'active' AND revision_id IN (SELECT id FROM plugin_revisions WHERE source_id = $2)")
        .bind(tenant_id).bind(source_id).execute(&mut **transaction).await?;
    Ok(())
}

pub(super) async fn start_instance(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    revision_id: &str,
    instance: Option<&ProcessInstance>,
) -> Result<()> {
    sqlx::query("INSERT INTO plugin_runtime_instances (id, tenant_id, revision_id, state, runtime_handle, endpoint, started_at) VALUES ($1, $2, $3, 'active', $4, $5, now())")
        .bind(uuid::Uuid::new_v4().to_string()).bind(tenant_id).bind(revision_id)
        .bind(instance.map(|instance| instance.instance_id.as_str()))
        .bind(instance.map(|instance| instance.endpoint.as_str()))
        .execute(&mut **transaction).await?;
    Ok(())
}

pub(super) fn runtime_name(runtime: PluginRuntime) -> &'static str {
    match runtime {
        PluginRuntime::PageDefinition => "page-definition",
        PluginRuntime::WasmComponent => "wasm-component",
        PluginRuntime::Process => "process",
        PluginRuntime::RustSource => "rust-source",
    }
}

pub(super) fn parse_runtime(value: String) -> Result<PluginRuntime> {
    match value.as_str() {
        "page-definition" => Ok(PluginRuntime::PageDefinition),
        "wasm-component" => Ok(PluginRuntime::WasmComponent),
        "process" => Ok(PluginRuntime::Process),
        "rust-source" => Ok(PluginRuntime::RustSource),
        _ => anyhow::bail!("未知插件运行时: {value}"),
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
