//! 持久化旧版安装同样允许在未装配执行器的宿主中恢复与管理。

use super::fixture::*;
use anyhow::{Context, Result};
use az_plugin_host::runtime::server::plugins::ManagementPlugin;
use serde_json::json;
use std::{any::TypeId, sync::Arc};
use uuid::Uuid;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn legacy_installations_suspend_without_executors_and_keep_static_pages() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let mut host = Host::start(root.path(), &database, &tenant, Default::default()).await?;
    let pool = host.state.service::<sqlx::PgPool>()?;
    let mut sources = Vec::new();
    for runtime in ["wasm-component", "process", "page-definition"] {
        let source = Uuid::new_v4().to_string();
        let revision = Uuid::new_v4().to_string();
        let page = format!("legacy-{source}");
        let manifest = json!({"runtime":{"kind":runtime,"artifact":"fixture"},
            "subplugins":[{"id":"fixture","pages":[page],"routes":["echo"],"account_actions":[page]}]});
        let pages = json!([{"id":page,"label":"Legacy", "icon":null,
            "scene":{"id":"account","label":"Account"},"body":{"kind":"frontend","entry":"index.html"}}]);
        sqlx::query("INSERT INTO plugin_sources(id,git) VALUES($1,$2)")
            .bind(&source)
            .bind(format!("https://github.com/example/{source}.git"))
            .execute(pool.as_ref())
            .await?;
        sqlx::query("INSERT INTO plugin_revisions(id,source_id,revision,runtime,manifest,pages) VALUES($1,$2,$1,$3,$4,$5)")
            .bind(&revision).bind(&source).bind(runtime).bind(manifest).bind(pages).execute(pool.as_ref()).await?;
        sqlx::query(
            "INSERT INTO tenant_plugin_bindings(tenant_id,source_id,revision_id) VALUES($1,$2,$3)",
        )
        .bind(&tenant)
        .bind(&source)
        .bind(&revision)
        .execute(pool.as_ref())
        .await?;
        sqlx::query("INSERT INTO plugin_runtime_instances(id,tenant_id,revision_id,state,runtime_handle,endpoint,started_at) VALUES($1,$2,$3,'active',$1,'http://127.0.0.1:1',now())")
            .bind(Uuid::new_v4().to_string()).bind(&tenant).bind(&revision).execute(pool.as_ref()).await?;
        sources.push((source, page, runtime));
    }
    let before: serde_json::Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(b) ORDER BY source_id) FROM tenant_plugin_bindings b WHERE tenant_id=$1")
        .bind(&tenant).fetch_one(pool.as_ref()).await?;
    let mut selected = host.selected.clone();
    selected.retain(|entry| entry.as_ref().type_id() != TypeId::of::<ManagementPlugin>());
    selected.push(Arc::new(ManagementPlugin));
    host.reconfigure(selected).await?;
    let after: serde_json::Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(b) ORDER BY source_id) FROM tenant_plugin_bindings b WHERE tenant_id=$1")
        .bind(&tenant).fetch_one(pool.as_ref()).await?;
    assert_eq!(before, after);
    let catalog = success(
        host.client
            .get(format!("{}/api/runtime/catalog", host.base))
            .send()
            .await?,
    )
    .await?;
    for (source, page, runtime) in sources {
        let visible = runtime == "page-definition";
        let expected = if visible { "active" } else { "suspended" };
        let data = &catalog["data"];
        let installed = data["plugins"]
            .as_array()
            .context("没有插件目录")?
            .iter()
            .find(|entry| entry["source_id"] == source)
            .context("没有旧版安装记录")?;
        assert_eq!(installed["state"], expected);
        assert_eq!(
            data["pages"]
                .as_array()
                .context("没有页面目录")?
                .iter()
                .any(|entry| entry["id"] == page),
            visible
        );
        assert_eq!(
            data["account_items"]
                .as_array()
                .context("没有账户目录")?
                .iter()
                .any(|entry| entry["page_id"] == page),
            visible
        );
        assert_eq!(data["page_versions"].get(&page).is_some(), visible);
        if !visible {
            assert_eq!(
                host.post("/api/runtime/frontend/mount", json!({"page_id":page}))
                    .await?
                    .status(),
                503
            );
            assert_eq!(
                host.post(
                    "/api/runtime/pages/action",
                    json!({"page_id":page,"action_id":"test"})
                )
                .await?
                .status(),
                503
            );
            assert_eq!(
                host.client
                    .get(format!("{}/api/runtime/services/{source}/echo", host.base))
                    .send()
                    .await?
                    .status(),
                503
            );
        }
        for action in ["disable", "uninstall"] {
            success(
                host.post(
                    &format!("/api/runtime/plugins/{source}/{action}"),
                    json!({}),
                )
                .await?,
            )
            .await?;
        }
    }
    host.stop().await?;
    Ok(())
}
