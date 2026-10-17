use super::*;

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn pairing_same_machine_revokes_previous_active_device() -> Result<()> {
    let root = tempfile::tempdir()?;
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("worker_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut isolated = reqwest::Url::parse(&database)?;
    isolated
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        isolated.as_ref(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let workers = state.service::<dyn RuntimeWorkerService>()?;
    let session = SessionContext {
        session_id: "session".into(),
        user_id: "owner".into(),
        account: "owner".into(),
        display_name: "owner".into(),
        tenant_id: "test".into(),
        tenant_label: "test".into(),
        permissions: vec![],
    };
    let request = PairRequest {
        label: "same-machine".into(),
        platform: "darwin".into(),
        capabilities: vec!["space.scan".into()],
        machine_id: Some("machine-aaa".into()),
    };
    let first = workers.pair(request.clone()).await?;
    workers.approve(&session, &first.code).await?;
    workers
        .enqueue(
            &session,
            SubmitTask {
                id: uuid::Uuid::new_v4().to_string(),
                worker_id: first.device_id.clone(),
                capability: "space.scan".into(),
                input: json!({}),
            },
        )
        .await?;
    // 同一 machine_id 重新配对：旧的 active 记录被撤销，只留一条。
    let second = workers.pair(request.clone()).await?;
    workers.approve(&session, &second.code).await?;
    let devices = workers.list(&session).await?;
    assert_eq!(
        devices
            .iter()
            .filter(|device| device.status != "revoked")
            .count(),
        1
    );
    assert_eq!(devices[0].id, second.device_id);
    let stale_state: String = sqlx::query_scalar("SELECT state FROM worker_devices WHERE id=$1")
        .bind(&first.device_id)
        .fetch_one(state.service::<sqlx::PgPool>()?.as_ref())
        .await?;
    assert_eq!(stale_state, "revoked");
    let task_state: String =
        sqlx::query_scalar("SELECT state FROM worker_tasks WHERE worker_id=$1")
            .bind(&first.device_id)
            .fetch_one(state.service::<sqlx::PgPool>()?.as_ref())
            .await?;
    assert_eq!(task_state, "cancelled");

    let other = workers
        .pair(PairRequest {
            // 故意与第一台同名同平台，但 machine_id 不同，必须各自保留。
            label: "same-machine".into(),
            platform: "darwin".into(),
            capabilities: vec!["space.scan".into()],
            machine_id: Some("machine-bbb".into()),
        })
        .await?;
    workers.approve(&session, &other.code).await?;
    let devices = workers.list(&session).await?;
    assert_eq!(
        devices
            .iter()
            .filter(|device| device.status != "revoked")
            .count(),
        2
    );

    state.service::<sqlx::PgPool>()?.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn terminal_sessions_single_owner_and_replacement() -> Result<()> {
    use super::model::CreateTerminal;

    let root = tempfile::tempdir()?;
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("worker_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut isolated = reqwest::Url::parse(&database)?;
    isolated
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        isolated.as_ref(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let workers = state.service::<dyn RuntimeWorkerService>()?;
    let session = SessionContext {
        session_id: "session".into(),
        user_id: "owner".into(),
        account: "owner".into(),
        display_name: "owner".into(),
        tenant_id: "test".into(),
        tenant_label: "test".into(),
        permissions: vec![],
    };
    let pairing = workers
        .pair(PairRequest {
            label: "terminal-host".into(),
            platform: "darwin".into(),
            capabilities: vec!["terminal.open".into()],
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
        })
        .await?;
    workers.approve(&session, &pairing.code).await?;
    let device = workers.identity(&pairing.token).await?;
    let create = |cols, rows| CreateTerminal {
        worker_id: pairing.device_id.clone(),
        cols,
        rows,
    };

    let first = workers.terminal_create(&session, create(100, 30)).await?;
    let claimed = workers
        .terminal_claim(&device, 0)
        .await?
        .context("首次终端会话未被领取")?;
    assert_eq!(claimed.id, first.id);

    let second = workers.terminal_create(&session, create(120, 40)).await?;
    let first_state: String =
        sqlx::query_scalar("SELECT state FROM worker_terminal_sessions WHERE id=$1")
            .bind(&first.id)
            .fetch_one(state.service::<sqlx::PgPool>()?.as_ref())
            .await?;
    assert_eq!(first_state, "closing");
    assert_eq!(second.state, "waiting");

    let replacement = workers
        .terminal_claim(&device, 0)
        .await?
        .context("替换终端未被领取")?;
    assert_eq!(replacement.id, second.id);
    assert!(workers.terminal_claim(&device, 0).await?.is_none());

    state.service::<sqlx::PgPool>()?.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
}
