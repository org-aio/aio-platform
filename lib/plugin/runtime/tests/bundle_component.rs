#![cfg(feature = "wasm")]

use az_plugin_contract::transport::Request;
use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{CapabilityGrants, RequestContext};
use az_plugin_runtime::{ComponentEngine, ComponentSlot, DatabaseProvisioner, InvocationResources};

fn context(tenant: &str) -> RequestContext {
    RequestContext {
        tenant_id: Some(tenant.into()),
        user_id: Some("tester".into()),
        request_id: "bundle-test".into(),
        ..Default::default()
    }
}

async fn count(slot: &ComponentSlot, digest: &str, method: &str, tenant: &str) -> Result<i64> {
    let response = slot
        .handle(
            digest,
            Request {
                method: method.into(),
                path: "/counter".into(),
                query: None,
                headers: vec![],
                body: vec![],
            },
            context(tenant),
        )
        .await?;
    ensure!(
        response.status == 200,
        "计数器失败: {}",
        String::from_utf8_lossy(&response.body)
    );
    let value: serde_json::Value = serde_json::from_slice(&response.body)?;
    assert_eq!(value["tenantId"], tenant);
    value["count"].as_i64().context("缺少 count")
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and built fullstack Kotlin repository"]
async fn fullstack_bundle_keeps_database_across_atomic_replacement() -> Result<()> {
    let root = PathBuf::from(std::env::var("AIO_TEST_KMP_ROOT")?);
    let commit = std::env::var("AIO_TEST_KMP_COMMIT")?;
    let build = |version: &str| -> Result<_> {
        let bundle = Bundle::from_directory(
            &root,
            "backend/component/aio-plugin.toml",
            "https://github.com/zjarlin/aio-plugin-kmp-example.git".into(),
            commit.clone(),
            version.into(),
        )?;
        Ok(Arc::new(Bundle::decode(&bundle.encode()?)?.verify()?))
    };
    let first = build("0.3.0")?;
    let second = build("0.3.1")?;
    assert_ne!(first.digest(), second.digest());
    assert!(first.frontend("counter.html").is_some());
    assert!(
        first.component().len() < 1024 * 1024,
        "Kotlin Component 不包含 JVM"
    );
    let migration = first
        .migrations()
        .map(|(_, sql)| sql)
        .collect::<Vec<_>>()
        .join("\n");
    let provisioner =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let source = uuid::Uuid::new_v4();
    let database = provisioner
        .create(&source.to_string(), "first", &migration)
        .await?;
    let resources = InvocationResources {
        database: Some(database),
        ..Default::default()
    };
    let grants = CapabilityGrants {
        database: true,
        ..Default::default()
    };
    let engine = ComponentEngine::new()?;
    let slot = ComponentSlot::new(source, "first".into(), "2026.9.11".parse()?)?;
    slot.replace(&engine, first.clone(), grants.clone(), resources.clone())
        .await?;
    assert_eq!(count(&slot, first.digest(), "GET", "first").await?, 0);
    assert_eq!(count(&slot, first.digest(), "POST", "first").await?, 1);

    let changed_root = tempfile::tempdir()?;
    for directory in ["dist/frontend", "backend/migrations"] {
        std::fs::create_dir_all(changed_root.path().join(directory))?;
    }
    std::fs::write(
        changed_root.path().join("aio-plugin.toml"),
        std::fs::read(root.join("backend/component/aio-plugin.toml"))?,
    )?;
    std::fs::write(
        changed_root.path().join("dist/plugin.wasm"),
        first.component(),
    )?;
    std::fs::write(
        changed_root.path().join("dist/frontend/counter.html"),
        first.frontend("counter.html").unwrap(),
    )?;
    std::fs::write(
        changed_root
            .path()
            .join("backend/migrations/0001_counter.sql"),
        format!("{migration}\nCREATE TABLE new_feature(id BIGINT);"),
    )?;
    let changed_schema = Bundle::from_directory(
        changed_root.path(),
        "aio-plugin.toml",
        "https://github.com/zjarlin/aio-plugin-kmp-example.git".into(),
        commit,
        "0.4.0".into(),
    )?
    .verify()?;
    let changed_schema = Arc::new(changed_schema);
    let error = slot
        .replace(
            &engine,
            changed_schema.clone(),
            grants.clone(),
            resources.clone(),
        )
        .await
        .expect_err("迁移变更应进入维护流程");
    assert!(error.to_string().contains("维护迁移"));
    assert_eq!(count(&slot, first.digest(), "GET", "first").await?, 1);

    assert!(
        slot.replace(
            &engine,
            second.clone(),
            Default::default(),
            resources.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(count(&slot, first.digest(), "GET", "first").await?, 1);
    slot.replace(&engine, second.clone(), grants.clone(), resources.clone())
        .await?;
    assert!(count(&slot, first.digest(), "GET", "first").await.is_err());
    assert_eq!(count(&slot, second.digest(), "GET", "first").await?, 1);
    assert_eq!(count(&slot, second.digest(), "POST", "first").await?, 2);
    assert!(count(&slot, second.digest(), "GET", "other").await.is_err());

    let other = ComponentSlot::new(source, "other".into(), "2026.9.11".parse()?)?;
    assert!(
        other
            .replace(&engine, first.clone(), grants.clone(), resources.clone())
            .await
            .is_err()
    );
    let other_database = provisioner
        .create(&source.to_string(), "other", &migration)
        .await?;
    other
        .replace(
            &engine,
            first.clone(),
            grants.clone(),
            InvocationResources {
                database: Some(other_database),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(count(&other, first.digest(), "GET", "other").await?, 0);

    slot.deactivate().await;
    assert!(slot.snapshot().await.is_none());
    assert!(
        slot.replace(&engine, changed_schema, grants.clone(), resources.clone())
            .await
            .is_err()
    );
    slot.replace(&engine, first.clone(), grants, resources)
        .await?;
    assert_eq!(count(&slot, first.digest(), "GET", "first").await?, 2);
    Ok(())
}
