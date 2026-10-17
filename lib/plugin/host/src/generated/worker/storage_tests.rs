use super::{WorkerService, WorkerServiceUnavailable, model::*};
use crate::{identity::SessionContext, runtime::server::RuntimeState};
use anyhow::Result;
use std::sync::Arc;

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn device_storage_contract_preserves_scope_atomic_keys_and_backend_errors() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("worker_storage_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut isolated = reqwest::Url::parse(&database)?;
    isolated
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let root = tempfile::tempdir()?;
    let state = RuntimeState::isolated_admin_test(
        Arc::new(super::tests::Identity),
        isolated.as_str(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let workers = state.service::<dyn WorkerService>()?;
    let session = SessionContext {
        session_id: "session".into(),
        tenant_id: "test".into(),
        user_id: "owner".into(),
        account: "owner".into(),
        display_name: "owner".into(),
        tenant_label: "test".into(),
        permissions: vec![],
    };
    let pair = workers
        .pair(PairRequest {
            label: "storage".into(),
            platform: "linux".into(),
            capabilities: vec!["space.scan".into()],
            machine_id: None,
        })
        .await?;
    workers.approve(&session, &pair.code).await?;
    let device = workers.identity(&pair.token).await?;
    assert!(
        workers
            .capability_active("test", "owner", &device.id, "space.scan")
            .await?
    );
    assert!(
        !workers
            .capability_active("other", "owner", &device.id, "space.scan")
            .await?
    );
    assert!(
        !workers
            .capability_active("test", "other", &device.id, "space.scan")
            .await?
    );
    assert!(
        !workers
            .capability_active("test", "owner", &device.id, "skills.sync")
            .await?
    );
    workers.skills_access(&device, true).await?;
    let foreign = DeviceIdentity {
        tenant: "other".into(),
        ..device.clone()
    };
    workers.skills_access(&foreign, false).await?;
    assert!(
        workers
            .capability_active("test", "owner", &device.id, "skills.sync")
            .await?
    );
    workers.skills_access(&device, false).await?;
    assert!(
        !workers
            .capability_active("test", "owner", &device.id, "skills.sync")
            .await?
    );
    assert!(
        workers
            .capability_active("test", "owner", &device.id, "space.scan")
            .await?
    );

    // 两个并发事务必须选定同一密文，后续候选不能覆盖已使用的归档密钥。
    let (left, right) = tokio::join!(
        workers.archive_key(&device, vec![1; 32]),
        workers.archive_key(&device, vec![2; 32]),
    );
    let left = left?;
    assert_eq!(left, right?);
    assert!(left == vec![1; 32] || left == vec![2; 32]);
    assert_eq!(workers.archive_key(&device, vec![3; 32]).await?, left);
    assert_eq!(
        workers.archive_key(&foreign, vec![4; 32]).await?,
        vec![4; 32]
    );

    workers.revoke(&session, &device.id).await?;
    assert!(
        !workers
            .capability_active("test", "owner", &device.id, "space.scan")
            .await?
    );
    assert!(
        !workers
            .identity(&pair.token)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("撤权应拒绝"))?
            .is::<WorkerServiceUnavailable>()
    );
    state.service::<sqlx::PgPool>()?.close().await;
    for error in [
        workers.identity(&pair.token).await.err(),
        workers
            .claim(&device, &uuid::Uuid::new_v4().to_string())
            .await
            .err(),
        workers.heartbeat(&device, None).await.err(),
        workers.terminal_access(&device, true).await.err(),
    ] {
        assert!(
            error
                .ok_or_else(|| anyhow::anyhow!("关闭后端应失败"))?
                .is::<WorkerServiceUnavailable>()
        );
    }
    state.shutdown().await?;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    admin.close().await;
    Ok(())
}
