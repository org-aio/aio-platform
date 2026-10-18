#[cfg(feature = "postgres")]
use anyhow::Context;
use anyhow::Result;
use az_plugin_contract::{CapabilityGrants, InvocationScope, RequestContext};

use crate::{bindings::aio::plugin::management, state::InvocationState};

fn context() -> RequestContext {
    RequestContext {
        tenant_id: Some("test".into()),
        user_id: Some("test".into()),
        session_id: None,
        request_id: "test".into(),
        permissions: Vec::new(),
    }
}

#[tokio::test]
async fn anonymous_management_is_rejected_even_when_capability_is_granted() -> Result<()> {
    use management::Host as _;
    let mut state = InvocationState::new(
        InvocationScope {
            source_id: "source".into(),
            revision: "revision".into(),
            context: RequestContext {
                user_id: None,
                ..context()
            },
            grants: CapabilityGrants {
                management: true,
                ..Default::default()
            },
        },
        Default::default(),
    );
    state.executing = true;
    let result = state
        .invoke(crate::bindings::aio::plugin::transport::Request {
            method: "POST".into(),
            path: "/install".into(),
            query: None,
            headers: vec![],
            body: vec![],
        })
        .await?;
    assert!(result.unwrap_err().contains("匿名"));
    Ok(())
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn unfinished_and_failed_transactions_are_rolled_back() -> Result<()> {
    use crate::{
        DatabaseProvisioner, InvocationResources,
        bindings::aio::plugin::database::{Host as _, Value},
    };
    let source = format!(
        "tx-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let database = DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?)
        .await?
        .create(
            &source,
            "test",
            "CREATE TABLE records (id BIGINT PRIMARY KEY, value TEXT NOT NULL)",
        )
        .await?;
    let scope = InvocationScope {
        source_id: source,
        revision: "test".into(),
        context: context(),
        grants: CapabilityGrants {
            database: true,
            ..Default::default()
        },
    };
    let mut state = InvocationState::new(
        scope,
        InvocationResources {
            database: Some(database.clone()),
            ..Default::default()
        },
    );
    state.executing = true;
    let id = state.begin().await?.map_err(anyhow::Error::msg)?;
    state
        .execute(
            id,
            "INSERT INTO records VALUES($1, $2)".into(),
            vec![Value::Integer(1), Value::Text("uncommitted".into())],
        )
        .await?
        .map_err(anyhow::Error::msg)?;
    state.cleanup().await?;
    state.executing = true;
    let id = state.begin().await?.map_err(anyhow::Error::msg)?;
    let rows = state
        .query(id, "SELECT * FROM records".into(), vec![])
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(rows.values.is_empty());
    state
        .execute(
            id,
            "INSERT INTO records VALUES(1, 'pending')".into(),
            vec![],
        )
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(
        state
            .query(id, "SELECT * FROM foreign_tenant.records".into(), vec![])
            .await?
            .is_err()
    );
    assert!(state.finish(id, true).await?.is_err());
    let mut locking = database.begin().await?;
    locking
        .execute("INSERT INTO records VALUES(1, 'locked')", vec![])
        .await?;
    let blocked = state.begin().await?.map_err(anyhow::Error::msg)?;
    let start = std::time::Instant::now();
    assert!(
        state
            .execute(
                blocked,
                "INSERT INTO records VALUES(1, 'blocked')".into(),
                vec![]
            )
            .await?
            .is_err()
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(4));
    assert!(state.finish(blocked, true).await?.is_err());
    locking.rollback().await?;
    let id = state.begin().await?.map_err(anyhow::Error::msg)?;
    let rows = state
        .query(id, "SELECT count(*) FROM records".into(), vec![])
        .await?
        .map_err(anyhow::Error::msg)?;
    let Value::Integer(count) = rows.values.first().context("missing count")?[0] else {
        anyhow::bail!("wrong count type");
    };
    assert_eq!(count, 0);
    state.cleanup().await?;
    Ok(())
}
