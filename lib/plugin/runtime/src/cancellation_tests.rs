use az_plugin_contract::transport::Request;
use std::time::Duration;

use anyhow::Result;
use az_plugin_contract::{CapabilityGrants, InvocationScope, RequestContext};

use crate::{ComponentEngine, DatabaseProvisioner, InvocationResources};

fn increment() -> Request {
    Request {
        method: "POST".into(),
        path: "/counter".into(),
        query: None,
        headers: vec![],
        body: vec![],
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL and the built Kotlin v2 component"]
async fn cancelled_calls_release_connections_without_dropping_the_instance_handle() -> Result<()> {
    let source = format!(
        "cancel-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let database = DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?)
        .await?
        .create(
            &source,
            "test",
            &std::fs::read_to_string(std::env::var("AIO_TEST_MIGRATION")?)?,
        )
        .await?;
    let context = RequestContext {
        tenant_id: Some("test".into()),
        user_id: Some("tester".into()),
        session_id: None,
        request_id: "cancellation".into(),
    };
    let scope = InvocationScope {
        source_id: source,
        revision: "test".into(),
        context: context.clone(),
        grants: CapabilityGrants {
            database: true,
            ..Default::default()
        },
    };
    let resources = InvocationResources {
        database: Some(database.clone()),
        ..Default::default()
    };
    let engine = ComponentEngine::new()?;
    let compiled = engine
        .compile(&std::fs::read(std::env::var("AIO_TEST_COMPONENT")?)?)
        .await?;
    let mut blocker = database.begin().await?;
    sqlx::query("INSERT INTO counter VALUES(1, 0)")
        .execute(&mut *blocker)
        .await?;
    let mut cancelled = Vec::new();
    for _ in 0..5 {
        let mut instance = engine
            .instantiate(&compiled, scope.clone(), resources.clone())
            .await?;
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            instance.handle(increment(), context.clone()),
        )
        .await;
        assert!(result.is_err(), "请求应被尚未提交的计数行阻塞");
        assert!(instance.health().await.is_err(), "取消后的实例不能复用");
        cancelled.push(instance);
    }
    blocker.rollback().await?;
    let mut replacement = engine.instantiate(&compiled, scope, resources).await?;
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        replacement.handle(increment(), context),
    )
    .await??;
    assert_eq!(response.status, 200);
    let value: serde_json::Value = serde_json::from_slice(&response.body)?;
    assert_eq!(value["count"], 1);
    assert_eq!(cancelled.len(), 5);
    Ok(())
}
