use std::{
    any::TypeId,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Context, Result};
use az_plugin_package::{PACKAGE_CONTENT_TYPE, PluginPackage};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    fixture::*,
    legacy_executor::{LegacyExecutor, LegacyPlugin},
};

fn package(git: &str, version: &str, valid: bool) -> Result<PluginPackage> {
    let pages = json!([{"id":"external","label":if valid {"External"} else {""},
        "icon":null,"scene":{"id":"tools","label":"Tools"},"required_permission":null,
        "body":{"kind":"text","title":"External","content":"Fixture"}}]);
    let bytes = format!("executor-fixture\n{pages}");
    PluginPackage::new(git.into(),version.into(),None,
        "[plugin.runtime]\nkind='wasm-component'\nartifact='plugin.wasm'\n[plugin.marketplace]\ntitle='External'\nsummary='Executor validation'\nlicense='MIT'\ntags=['test']\n[[plugin.subplugins]]\nid='external'\npages=['external']\nroutes=['echo']\n".into(),
        bytes.as_bytes(),Default::default())
}

async fn publish(host: &Host, package: &PluginPackage, expected: &str) -> Result<Value> {
    let queued = success(
        host.client
            .post(format!("{}/api/runtime/plugins/publish", host.base))
            .header("content-type", PACKAGE_CONTENT_TYPE)
            .body(package.encode()?)
            .send()
            .await?,
    )
    .await?;
    let job = queued["data"]["job_id"].as_str().context("发布任务缺失")?;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let result = success(
                host.client
                    .get(format!("{}/api/runtime/publish-jobs/{job}", host.base))
                    .send()
                    .await?,
            )
            .await?;
            let state = result["data"]["state"].as_str().context("发布状态缺失")?;
            if matches!(state, "active" | "failed") {
                assert_eq!(state, expected, "{result}");
                return Ok::<_, anyhow::Error>(result);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}

async fn call(host: &Host, source: &str) -> Result<reqwest::Response> {
    Ok(host
        .client
        .get(format!("{}/api/runtime/services/{source}/echo", host.base))
        .send()
        .await?)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn selected_legacy_executor_owns_validation_installation_replacement_and_recovery()
-> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/legacy-{tenant}.git");
    let mut host = Host::start(root.path(), &database, &tenant, Default::default()).await?;
    let unavailable = package(&git, "1.0.0", true)?;
    let failed = publish(&host, &unavailable, "failed").await?;
    assert!(
        failed["data"]["detail"]
            .as_str()
            .context("失败详情缺失")?
            .contains("未安装 Wasm")
    );

    let base = host.selected.clone();
    let mut first_selection = base.clone();
    first_selection.push(Arc::new(LegacyPlugin::<1>));
    host.reconfigure(first_selection).await?;
    let first_executor = host.state.service::<LegacyExecutor<1>>()?;
    let first = package(&git, "1.0.1", true)?;
    let active = publish(&host, &first, "active").await?;
    let source = active["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .to_owned();
    let result = success(call(&host, &source).await?).await?;
    assert_eq!(result["executor"], 1);
    assert_eq!(result["revision"], first.rev);
    assert!(first_executor.validations.load(Ordering::SeqCst) >= 2);
    assert!(
        !host
            .client
            .get(format!("{}/api/runtime/services/{source}/echo", host.base))
            .header("cookie", "session=other")
            .send()
            .await?
            .status()
            .is_success()
    );

    // 执行器给出非法页面也不能破坏已经工作的版本。
    let invalid = package(&git, "2.0.0", false)?;
    publish(&host, &invalid, "failed").await?;
    assert_eq!(
        success(call(&host, &source).await?).await?["revision"],
        first.rev
    );
    let old_calls = first_executor.validations.load(Ordering::SeqCst);

    host.reconfigure(base.clone()).await?;
    assert_eq!(call(&host, &source).await?.status(), 503);
    assert!(
        !host
            .post(
                "/api/runtime/plugins/install",
                json!({"git":git,"rev":first.rev})
            )
            .await?
            .status()
            .is_success()
    );
    let catalog = success(
        host.client
            .get(format!("{}/api/runtime/catalog", host.base))
            .send()
            .await?,
    )
    .await?;
    let entry = catalog["data"]["plugins"]
        .as_array()
        .context("目录缺失")?
        .iter()
        .find(|entry| entry["source_id"] == source)
        .context("暂停记录丢失")?;
    assert_eq!(entry["state"], "suspended");
    assert_eq!(entry["revision"], first.rev);

    let mut second_selection = base;
    second_selection.push(Arc::new(LegacyPlugin::<2>));
    host.reconfigure(second_selection.clone()).await?;
    let second_executor = host.state.service::<LegacyExecutor<2>>()?;
    assert_eq!(success(call(&host, &source).await?).await?["executor"], 2);
    let third = package(&git, "3.0.0", true)?;
    publish(&host, &third, "active").await?;
    let result = success(call(&host, &source).await?).await?;
    assert_eq!(result["executor"], 2);
    assert_eq!(result["revision"], third.rev);
    assert_eq!(first_executor.validations.load(Ordering::SeqCst), old_calls);
    assert!(second_executor.validations.load(Ordering::SeqCst) >= 3);

    let mut duplicate = second_selection.clone();
    duplicate.push(Arc::new(LegacyPlugin::<1>));
    assert!(host.reconfigure(duplicate).await.is_err());
    assert_eq!(success(call(&host, &source).await?).await?["executor"], 2);
    assert!(
        !host
            .selected
            .iter()
            .any(|plugin| plugin.as_ref().type_id() == TypeId::of::<LegacyPlugin<1>>())
    );
    host.stop().await?;

    let host = Host::start_selected(root.path(), &database, second_selection).await?;
    let restored = success(call(&host, &source).await?).await?;
    assert_eq!(restored["revision"], third.rev);
    assert_eq!(restored["executor"], 2);
    host.stop().await?;
    Ok(())
}
