//! 真实 HTTP 生命周期使用替代文件目录，数据库配置故意无效。
mod delivery;
mod delivery_http;
mod delivery_postgres;
mod file;
mod installations;
mod publications;
mod queries;
mod recovery;

use super::{
    fixture::{Host, Identity, success},
    identity_support,
    legacy_executor::{LegacyExecutor, LegacyPlugin},
};
use anyhow::{Context, Result};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_host::runtime::{
    PageBody,
    server::{plugin_catalog::*, plugins::*},
};
use az_plugin_package::{PACKAGE_CONTENT_TYPE, PluginPackage};
use file::FileCatalog;
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

struct FilePlugin(Arc<FileCatalog>);
impl Plugin<RuntimeAssembly> for FilePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn PluginCatalog, FileCatalog>();
            Ok(())
        })
    }
}
fn selected(catalog: Arc<FileCatalog>) -> Vec<DynPlugin<RuntimeAssembly>> {
    identity_support::with_identity(
        Arc::new(Identity("tenant".into())),
        vec![
            Arc::new(ManagementPlugin),
            Arc::new(NavigationPlugin),
            Arc::new(FrontendPlugin),
            Arc::new(BootstrapPlugin),
            Arc::new(RepositoryPlugin),
            Arc::new(FilePlugin(catalog)),
            Arc::new(LegacyPlugin::<7>),
        ],
    )
}
fn package(version: &str) -> Result<PluginPackage> {
    let page = json!([{"id":"legacy-file","label":"File catalog","icon":null,"scene":{"id":"tools","label":"Tools"},"required_permission":null,"body":{"kind":"text","title":"File","content":version}}]);
    PluginPackage::new("https://github.com/example/legacy-file.git".into(),version.into(),Some("a".repeat(40)),
        "[plugin.runtime]\nkind='wasm-component'\nartifact='plugin.wasm'\n[plugin.marketplace]\ntitle='File catalog'\nsummary='Persistence replacement'\nlicense='MIT'\ntags=['test']\n[[plugin.subplugins]]\nid='file'\npages=['legacy-file']\nroutes=['echo']\n".into(),format!("executor-fixture\n{page}").as_bytes(),Default::default())
}
async fn publish(host: &Host, package: &PluginPackage) -> Result<Value> {
    let queued = success(
        host.client
            .post(format!("{}/api/runtime/plugins/publish", host.base))
            .header("content-type", PACKAGE_CONTENT_TYPE)
            .body(package.encode()?)
            .send()
            .await?,
    )
    .await?;
    let id = queued["data"]["job_id"].as_str().context("任务缺失")?;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let result = success(
                host.client
                    .get(format!("{}/api/runtime/publish-jobs/{id}", host.base))
                    .send()
                    .await?,
            )
            .await?;
            if result["data"]["state"] == "active" {
                return Ok(result);
            }
            anyhow::ensure!(result["data"]["state"] != "failed", "发布失败: {result}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}
async fn get(host: &Host, path: &str) -> Result<Value> {
    success(
        host.client
            .get(format!("{}{path}", host.base))
            .send()
            .await?,
    )
    .await
}
async fn change(host: &Host, source: &str, action: &str) -> Result<Value> {
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/{action}"),
            json!({}),
        )
        .await?,
    )
    .await
}

#[tokio::test]
async fn file_catalog_management_survives_restart_and_rejects_failed_commits_without_postgres()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("legacy-catalog.json");
    let catalog = FileCatalog::open(&path);
    let host =
        Host::start_selected(root.path(), "invalid database", selected(catalog.clone())).await?;
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    assert!(host.state.service::<PluginStore>().is_err());
    assert!(host.state.service::<dyn ToolCatalog>().is_err());
    let first = package("1.0.0")?;
    let active = publish(&host, &first).await?;
    let source = active["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .to_owned();
    let call = format!("/api/runtime/services/{source}/echo");
    assert_eq!(get(&host, &call).await?["revision"], first.rev);
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["pages"][0]["id"],
        "legacy-file"
    );
    assert_eq!(
        get(&host, "/api/runtime/marketplace").await?["data"]
            .as_array()
            .context("市场缺失")?
            .len(),
        1
    );
    assert!(catalog.bound_runtime("other", &source).await.is_err());
    assert!(
        catalog
            .active_page_binding("other", "legacy-file")
            .await?
            .is_none()
    );
    assert!(
        catalog
            .set_enabled("other", &source, false, None)
            .await
            .is_err()
    );
    let downloaded = host
        .client
        .get(format!("{}/api/runtime/packages/{}", host.base, first.rev))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    assert_eq!(PluginPackage::decode(&downloaded)?.rev, first.rev);
    let credential = success(
        host.post("/api/runtime/publish-credentials", json!({"git":first.git}))
            .await?,
    )
    .await?;
    let token = credential["data"]["token"].as_str().context("凭据缺失")?;
    assert!(catalog.publisher_binding(token).await?.is_some());
    assert!(!std::fs::read_to_string(&path)?.contains(token));

    let binding = catalog
        .active_page_binding("tenant", "legacy-file")
        .await?
        .context("页面缺失")?;
    let body: PageBody =
        serde_json::from_value(json!({"kind":"text","title":"Persisted","content":"saved"}))?;
    catalog
        .save_page_state(
            "tenant",
            &source,
            &binding.revision_id,
            "legacy-file",
            None,
            &body,
        )
        .await?;
    assert!(
        catalog
            .save_page_state(
                "tenant",
                &source,
                &binding.revision_id,
                "legacy-file",
                None,
                &body
            )
            .await
            .is_err()
    );
    change(&host, &source, "hide-menu").await?;
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["hidden_pages"],
        json!(["legacy-file"])
    );
    assert_eq!(get(&host, &call).await?["revision"], first.rev);

    let second = package("2.0.0")?;
    publish(&host, &second).await?;
    change(&host, &source, "rollback").await?;
    assert_eq!(get(&host, &call).await?["revision"], first.rev);
    assert!(
        !catalog
            .upgrade_eligible("tenant", &source, &second.rev)
            .await?
    );
    // 运行时已准备候选后模拟持久提交失败，旧实例必须继续服务且候选被清理。
    catalog.fail_activation.store(true, Ordering::SeqCst);
    let failed = host
        .post(
            "/api/runtime/plugins/install",
            json!({"git":second.git,"rev":second.rev}),
        )
        .await?;
    assert!(!failed.status().is_success());
    assert_eq!(get(&host, &call).await?["revision"], first.rev);
    let executor = host.state.service::<LegacyExecutor<7>>()?;
    assert!(!executor.deactivate("tenant", &source, &second.rev)?);
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["pages"][0]["body"]["content"],
        "saved"
    );
    host.stop().await?;
    drop(catalog);

    let catalog = FileCatalog::open(&path);
    let host =
        Host::start_selected(root.path(), "invalid database", selected(catalog.clone())).await?;
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    assert_eq!(get(&host, &call).await?["revision"], first.rev);
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["hidden_pages"],
        json!(["legacy-file"])
    );
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["pages"][0]["body"]["content"],
        "saved"
    );
    let releases = get(&host, &format!("/api/runtime/releases?git={}", first.git)).await?;
    assert_eq!(releases.as_array().context("发布列表缺失")?.len(), 2);
    change(&host, &source, "disable").await?;
    assert!(
        !host
            .client
            .get(format!("{}{call}", host.base))
            .send()
            .await?
            .status()
            .is_success()
    );
    host.stop().await?;
    drop(catalog);

    let catalog = FileCatalog::open(&path);
    let host =
        Host::start_selected(root.path(), "invalid database", selected(catalog.clone())).await?;
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["plugins"][0]["state"],
        "disabled"
    );
    change(&host, &source, "enable").await?;
    assert_eq!(get(&host, &call).await?["revision"], second.rev);
    change(&host, &source, "show-menu").await?;
    let removed = host
        .client
        .delete(format!("{}/api/runtime/marketplace", host.base))
        .json(&json!({"git":first.git}))
        .send()
        .await?;
    success(removed).await?;
    assert_eq!(
        get(&host, "/api/runtime/marketplace").await?["data"]
            .as_array()
            .context("目录缺失")?
            .len(),
        1
    );
    change(&host, &source, "uninstall").await?;
    assert!(
        get(&host, "/api/runtime/catalog").await?["data"]["plugins"]
            .as_array()
            .context("目录缺失")?
            .is_empty()
    );
    assert!(
        get(&host, "/api/runtime/marketplace").await?["data"]
            .as_array()
            .context("目录缺失")?
            .is_empty()
    );
    assert!(
        !host
            .post("/api/runtime/plugins/install", json!({"git":first.git}))
            .await?
            .status()
            .is_success()
    );
    let events = get(&host, &format!("/api/runtime/plugins/{source}/events")).await?;
    assert!(
        events["data"]
            .as_array()
            .context("事件缺失")?
            .iter()
            .any(|e| e["lifecycle"] == "uninstall")
    );
    host.stop().await?;
    assert!(!FileCatalog::open(&path).has_plugins("tenant").await?);
    Ok(())
}
