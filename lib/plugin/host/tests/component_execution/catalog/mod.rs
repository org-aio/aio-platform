//! 目录替换验收：文件持久化、真实 HTTP 生命周期和完全不连接 PostgreSQL 的最小组合。
mod file;
#[cfg(feature = "postgres")]
mod postgres;
mod resources;
mod streaming;

use super::{fixture::*, identity_support};
use anyhow::{Context, Result};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_host::runtime::server::plugins::*;
use file::FileCatalog;
use serde_json::json;
use std::any::TypeId;
use std::sync::{Arc, atomic::Ordering};
use uuid::Uuid;

struct CatalogPlugin(Arc<FileCatalog>);
impl Plugin<RuntimeAssembly> for CatalogPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn ComponentCatalog, FileCatalog>();
            Ok(())
        })
    }
}
fn selected(
    root: &std::path::Path,
    catalog: Arc<FileCatalog>,
    tenant: &str,
    probes: [Arc<Probe>; 2],
    surfaces: bool,
) -> Vec<DynPlugin<RuntimeAssembly>> {
    selected_with_identity(
        root,
        catalog,
        probes,
        surfaces,
        Arc::new(Identity(tenant.into())),
    )
}
fn selected_with_identity<P>(
    root: &std::path::Path,
    catalog: Arc<FileCatalog>,
    probes: [Arc<Probe>; 2],
    surfaces: bool,
    identity: Arc<P>,
) -> Vec<DynPlugin<RuntimeAssembly>>
where
    P: az_plugin_host::identity::IdentityProvider
        + az_plugin_host::authorization::AuthorizationProvider,
{
    let mut selected = plugins(probes, surfaces);
    #[cfg(feature = "postgres")]
    selected.retain(|plugin| {
        ![
            TypeId::of::<PostgresComponentCatalogPlugin>(),
            TypeId::of::<PostgresComponentPlugin>(),
            TypeId::of::<StorePlugin>(),
            TypeId::of::<DatabasePlugin>(),
            TypeId::of::<PostgresPlugin>(),
        ]
        .contains(&plugin.as_ref().type_id())
    });
    if surfaces {
        selected.push(Arc::new(super::legacy_catalog::FilePlugin(
            super::legacy_catalog::file::FileCatalog::open(&root.join("legacy.json")),
        )));
    }
    selected.push(Arc::new(CatalogPlugin(catalog)));
    selected.push(Arc::new(resources::NoDatabase));
    identity_support::with_identity(identity, selected)
}
fn bootstrap(
    root: &std::path::Path,
    bundle: &az_plugin_bundle::Bundle,
    tenant: &str,
) -> Result<BootstrapComponent> {
    let path = root.join("bootstrap.aio");
    std::fs::write(&path, bundle.encode()?)?;
    Ok(BootstrapComponent {
        path,
        digest: bundle.digest.clone(),
        tenants: vec![tenant.into()],
    })
}

#[tokio::test]
async fn file_catalog_bootstraps_restarts_and_preserves_uninstall_without_postgres() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("catalog.json");
    let catalog = FileCatalog::open(&path);
    let bundle = package(
        root.path(),
        "https://github.com/example/bootstrap-file.git",
        1,
        "1.0.0",
    )?;
    let declaration = bootstrap(root.path(), &bundle, "tenant")?;
    let probes: [Arc<Probe>; 2] = Default::default();
    let mut selection = selected(
        root.path(),
        catalog.clone(),
        "tenant",
        probes.clone(),
        false,
    );
    selection.push(Arc::new(ComponentBootstrapPlugin(vec![
        declaration.clone(),
    ])));
    let host = Host::start_selected(root.path(), "invalid database", selection).await?;
    #[cfg(feature = "postgres")]
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    #[cfg(feature = "postgres")]
    assert!(probes[0].pool.lock().await.is_none());
    let source = catalog
        .source(&bundle.git)
        .await?
        .context("首次引导未发布")?
        .id;
    assert!(catalog.initialized("tenant", &bundle.git).await?);
    assert!(catalog.installation("other", source).await?.is_none());
    assert_eq!(
        probes[0]
            .active
            .lock()
            .await
            .get(&(source, "tenant".into()))
            .context("未激活")?
            .digest(),
        bundle.digest
    );
    let generation = catalog
        .installation("tenant", source)
        .await?
        .context("未安装")?
        .installation
        .context("安装记录缺失")?
        .generation;
    host.stop().await?;

    // 丢弃原目录实例后从磁盘重新读取，不借助进程内缓存恢复。
    drop(catalog);
    let catalog = FileCatalog::open(&path);
    let mut selection = selected(
        root.path(),
        catalog.clone(),
        "tenant",
        probes.clone(),
        false,
    );
    selection.push(Arc::new(ComponentBootstrapPlugin(vec![
        declaration.clone(),
    ])));
    let host = Host::start_selected(root.path(), "invalid database", selection).await?;
    assert_eq!(
        catalog
            .installation("tenant", source)
            .await?
            .context("重启丢失安装")?
            .installation
            .context("安装记录缺失")?
            .generation,
        generation
    );
    assert!(
        probes[0]
            .active
            .lock()
            .await
            .contains_key(&(source, "tenant".into()))
    );
    host.stop().await?;
    catalog.remove("tenant", source, generation, true).await?;
    drop(catalog);
    let catalog = FileCatalog::open(&path);
    let mut selection = selected(
        root.path(),
        catalog.clone(),
        "tenant",
        probes.clone(),
        false,
    );
    selection.push(Arc::new(ComponentBootstrapPlugin(vec![declaration])));
    let host = Host::start_selected(root.path(), "invalid database", selection).await?;
    assert!(catalog.installation("tenant", source).await?.is_none());
    assert!(catalog.initialized("tenant", &bundle.git).await?);
    assert!(probes[0].active.lock().await.is_empty());
    host.stop().await?;
    Ok(())
}

#[tokio::test]
async fn replacement_catalog_drives_http_lifecycle_without_database() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("catalog.json");
    let catalog = FileCatalog::open(&path);
    let tenant = "file-tenant";
    let probes: [Arc<Probe>; 2] = Default::default();
    let git = "https://github.com/example/file-catalog.git";
    let first = package(root.path(), git, 1, "1.0.0")?;
    let second = package(root.path(), git, 2, "2.0.0")?;
    let mut host = Host::start_selected(
        root.path(),
        "invalid database",
        selected(root.path(), catalog.clone(), tenant, probes.clone(), true),
    )
    .await?;
    let source: Uuid = success(host.publish(&first).await?).await?["data"]["source_id"]
        .as_str()
        .context("没有来源")?
        .parse()?;
    catalog.reject_install.store(true, Ordering::SeqCst);
    assert!(!host.install(&first).await?.status().is_success());
    assert!(!catalog.initialized(tenant, git).await?);
    assert!(catalog.installation(tenant, source).await?.is_none());
    catalog.reject_install.store(false, Ordering::SeqCst);
    success(host.install(&first).await?).await?;
    let token = host.assert_call(source, 1, &first).await?;
    success(host.publish(&second).await?).await?;
    catalog.reject_install.store(true, Ordering::SeqCst);
    assert!(!host.install(&second).await?.status().is_success());
    host.assert_call(source, 1, &first).await?;
    assert!(probes[1].active.lock().await.is_empty());
    catalog.reject_install.store(false, Ordering::SeqCst);
    success(host.install(&second).await?).await?;
    assert!(!host.invoke(&token).await?.status().is_success());
    host.assert_call(source, 2, &second).await?;
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/rollback"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.assert_call(source, 1, &first).await?;
    assert!(
        catalog
            .upgrade_target(tenant, source, &second.digest)
            .await?
            .is_none()
    );
    success(
        host.post(&format!("/api/runtime/plugins/{source}/disable"), json!({}))
            .await?,
    )
    .await?;
    assert!(host.mount(source).await.is_err());
    success(
        host.post(&format!("/api/runtime/plugins/{source}/enable"), json!({}))
            .await?,
    )
    .await?;
    host.assert_call(source, 1, &first).await?;
    let current = catalog
        .installation(tenant, source)
        .await?
        .context("缺少安装")?
        .installation
        .context("缺少安装记录")?;
    assert_eq!(
        current.excluded_digest.as_deref(),
        Some(second.digest.as_str())
    );
    assert!(
        catalog
            .install(ComponentInstallationCommit {
                tenant: tenant.into(),
                source,
                digest: second.digest.clone(),
                excluded_digest: None,
                expected_generation: None
            })
            .await
            .is_err()
    );
    assert!(
        catalog
            .remove(tenant, source, Uuid::new_v4(), true)
            .await
            .is_err()
    );
    let first_key = catalog.process_key(source, tenant, vec![1]).await?;
    assert_eq!(
        catalog.process_key(source, tenant, vec![2]).await?,
        first_key
    );
    catalog
        .record_process_actor(source, tenant, "owner")
        .await?;
    assert!(
        !catalog
            .process_actor_exists(source, "other", "owner")
            .await?
    );
    let archived = success(
        host.client
            .get(format!("{}/api/runtime/releases?git={git}", host.base))
            .send()
            .await?,
    )
    .await?;
    assert!(archived.to_string().contains(&second.digest));
    #[cfg(feature = "postgres")]
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    // 只替换目录插件，依赖它的资源和运行实例必须自动重建。
    let replacement = FileCatalog::open(&path);
    let before = probes[0].stopped.load(Ordering::SeqCst);
    let next = host
        .selected
        .iter()
        .map(|plugin| {
            if plugin.as_ref().type_id() == TypeId::of::<CatalogPlugin>() {
                Arc::new(CatalogPlugin(replacement.clone())) as DynPlugin<RuntimeAssembly>
            } else {
                plugin.clone()
            }
        })
        .collect();
    host.reconfigure(next).await?;
    assert!(probes[0].stopped.load(Ordering::SeqCst) > before);
    let expected: Arc<dyn ComponentCatalog> = replacement;
    assert!(Arc::ptr_eq(
        &host.state.service::<dyn ComponentCatalog>()?,
        &expected
    ));
    host.assert_call(source, 1, &first).await?;
    let other = selected(
        root.path(),
        catalog.clone(),
        "other",
        Default::default(),
        true,
    );
    host.reconfigure(other).await?;
    assert!(host.mount(source).await.is_err());
    host.stop().await?;
    drop(catalog);
    let reopened = FileCatalog::open(&path);
    let host = Host::start_selected(
        root.path(),
        "invalid database",
        selected(root.path(), reopened.clone(), tenant, probes, true),
    )
    .await?;
    host.assert_call(source, 1, &first).await?;
    assert_eq!(
        reopened.process_key(source, tenant, vec![3]).await?,
        first_key
    );
    assert!(
        reopened
            .process_actor_exists(source, tenant, "owner")
            .await?
    );
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/uninstall"),
            json!({}),
        )
        .await?,
    )
    .await?;
    assert!(reopened.installation(tenant, source).await?.is_none());
    assert!(reopened.initialized(tenant, git).await?);
    host.stop().await?;
    Ok(())
}
