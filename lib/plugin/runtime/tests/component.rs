#![cfg(all(feature = "wasm", feature = "postgres"))]

use anyhow::{Context, Result};
use az_plugin_contract::transport::Request;
use az_plugin_contract::{CapabilityGrants, InvocationScope, RequestContext};
use az_plugin_runtime::{
    ComponentEngine, ComponentInstance, DatabaseProvisioner, InvocationResources,
    bindings::aio::plugin::transport::Phase,
};

fn context(tenant: &str) -> RequestContext {
    RequestContext {
        tenant_id: Some(tenant.into()),
        user_id: Some("test-user".into()),
        session_id: None,
        request_id: "integration".into(),
        permissions: Vec::new(),
    }
}

async fn count(instance: &mut ComponentInstance, tenant: &str, method: &str) -> Result<i64> {
    let response = instance
        .handle(
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
    anyhow::ensure!(
        response.status == 200,
        "响应 {}: {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    );
    let body: serde_json::Value = serde_json::from_slice(&response.body)?;
    assert_eq!(body["tenantId"], tenant);
    body["count"].as_i64().context("缺少计数值")
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database and the built Kotlin v2 component"]
async fn kotlin_component_persists_across_replacement_and_isolates_tenants() -> Result<()> {
    let component = std::fs::read(std::env::var("AIO_TEST_COMPONENT")?)?;
    let migration = std::fs::read_to_string(std::env::var("AIO_TEST_MIGRATION")?)?;
    let provisioner =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let source = format!(
        "kmp-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let database = provisioner.create(&source, "first", &migration).await?;
    let second_database = provisioner.create(&source, "second", &migration).await?;
    let scope = InvocationScope {
        source_id: source.clone(),
        revision: "test-v2".into(),
        context: context("first"),
        grants: CapabilityGrants {
            database: true,
            ..Default::default()
        },
    };
    let resources = InvocationResources {
        database: Some(database),
        ..Default::default()
    };
    let engine = ComponentEngine::new()?;
    let component = engine.compile(&component).await?;
    let mut instance = engine
        .instantiate(&component, scope.clone(), resources.clone())
        .await?;
    assert_eq!(instance.describe().await?.pages[0].entry, "counter.html");
    instance.health().await?;
    instance.lifecycle(Phase::Prepare).await?;
    instance.lifecycle(Phase::Activate).await?;
    assert_eq!(count(&mut instance, "first", "GET").await?, 0);
    assert_eq!(count(&mut instance, "first", "POST").await?, 1);
    assert!(count(&mut instance, "second", "GET").await.is_err());
    instance.lifecycle(Phase::Deactivate).await?;
    drop(instance);
    let mut replacement = engine
        .instantiate(&component, scope.clone(), resources.clone())
        .await?;
    assert_eq!(count(&mut replacement, "first", "GET").await?, 1);
    assert_eq!(count(&mut replacement, "first", "POST").await?, 2);
    let second_scope = InvocationScope {
        context: context("second"),
        ..scope.clone()
    };
    assert!(
        engine
            .instantiate(&component, second_scope.clone(), resources.clone())
            .await
            .is_err()
    );
    let mut second = engine
        .instantiate(
            &component,
            second_scope,
            InvocationResources {
                database: Some(second_database),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(count(&mut second, "second", "GET").await?, 0);
    let denied_scope = InvocationScope {
        grants: Default::default(),
        ..scope
    };
    let mut denied = engine
        .instantiate(&component, denied_scope, resources)
        .await?;
    assert!(count(&mut denied, "first", "POST").await.is_err());
    assert_eq!(count(&mut replacement, "first", "GET").await?, 2);
    Ok(())
}
