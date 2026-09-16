use super::*;
use crate::{
    generated::worker::{
        controller,
        model::{CompleteTask, PairRequest, Task},
    },
    identity::IdentityProvider,
    runtime::server::RuntimeState,
};
use sqlx::Row;
use uuid::Uuid;

struct Identity;

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(user) = headers
            .get("x-test-user")
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(None);
        };
        let tenant = headers
            .get("x-test-tenant")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("test");
        Ok(Some(session(tenant, user)))
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, _: &str, user: &str) -> Result<bool> {
        Ok(user != "disabled")
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
}

fn session(tenant: &str, user: &str) -> SessionContext {
    SessionContext {
        session_id: user.into(),
        tenant_id: tenant.into(),
        user_id: user.into(),
        account: user.into(),
        display_name: user.into(),
        tenant_label: tenant.into(),
        permissions: vec![],
    }
}

fn submit(worker: &str, capability: &str, input: Value) -> SubmitTask {
    SubmitTask {
        id: Uuid::new_v4().to_string(),
        worker_id: worker.into(),
        capability: capability.into(),
        input,
    }
}

fn bridge(operation: &str, capability: &str) -> Value {
    json!({"tenantId":"test", "userId":"owner", "operation":operation, "capability":capability})
}

async fn call(gateway: &Gateway, headers: &HeaderMap, request: Value) -> Result<Value> {
    execute(gateway, headers, serde_json::from_value(request)?).await
}

#[test]
fn applications_do_not_accept_shell_or_paths() {
    for value in ["Postman", "Google Chrome", "com.postmanlabs.mac", "备忘录"] {
        assert!(validate_application(value).is_ok());
    }
    for value in [
        "",
        "--args",
        "/Applications/Postman.app",
        "Postman; curl evil",
        "$(id)",
        "Postman\nopen Terminal",
    ] {
        assert!(validate_application(value).is_err());
    }
}

#[test]
fn workspace_discovery_intersects_process_grants() -> Result<()> {
    let grants = vec![DESKTOP.into(), WORKSPACE.into(), "shell.execute".into()];
    assert_eq!(
        requested_capabilities(&grants, "*", "list")?,
        [DESKTOP, WORKSPACE]
    );
    assert_eq!(
        requested_capabilities(&grants, WORKSPACE, "submit")?,
        [WORKSPACE]
    );
    assert!(requested_capabilities(&grants, "*", "task").is_err());
    assert!(requested_capabilities(&grants, "*", "cancel").is_err());
    assert!(requested_capabilities(&grants, "shell.execute", "list").is_err());
    assert!(requested_capabilities(&[DESKTOP.into()], WORKSPACE, "submit").is_err());
    assert!(requested_capabilities(&[], "*", "list").is_err());
    let request: Request =
        serde_json::from_value(json!({"tenantId":"test","userId":"owner","operation":"list"}))?;
    assert!(request.capability.is_none());
    Ok(())
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn workspace_access_cancel_and_bridge_isolation() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("workspace_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut database = reqwest::Url::parse(&database)?;
    database
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let root = tempfile::tempdir()?;
    let mut state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        database.as_str(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    state.components = Some(
        crate::runtime::server::components::Components::open(
            state.store.pool.clone(),
            database.as_str(),
            &root.path().join("components-keyring.json"),
            root.path().join("objects"),
            state.identity.clone(),
        )
        .await?,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/api/runtime/workers", listener.local_addr()?);
    let server = tokio::spawn(
        axum::serve(listener, controller::router().with_state(state.clone())).into_future(),
    );
    let result = verify_workspace(&state, &base).await;
    server.abort();
    state.store.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result
}

async fn verify_workspace(state: &RuntimeState, base: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let owner = session("test", "owner");
    let pair = state
        .workers
        .pair(PairRequest {
            label: "workspace-device".into(),
            platform: "linux".into(),
            capabilities: vec!["space.scan".into()],
        })
        .await?;
    state.workers.approve(&owner, &pair.code).await?;
    let input = json!({"action":"describe"});
    assert!(
        state
            .workers
            .enqueue(&owner, submit(&pair.device_id, WORKSPACE, input.clone()))
            .await
            .is_err()
    );
    let access = format!("{base}/workspaces/access");
    assert_eq!(
        client
            .post(&access)
            .header("x-test-user", "owner")
            .json(&json!({"enabled":true}))
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .post(&access)
            .basic_auth("worker", Some(&pair.token))
            .json(&json!({"enabled":true}))
            .send()
            .await?
            .status(),
        401
    );
    assert!(
        client
            .post(&access)
            .bearer_auth(&pair.token)
            .json(&json!({"enabled":true,"capability":"shell.execute"}))
            .send()
            .await?
            .status()
            .is_client_error()
    );
    for _ in 0..2 {
        client
            .post(&access)
            .bearer_auth(&pair.token)
            .json(&json!({"enabled":true}))
            .send()
            .await?
            .error_for_status()?;
    }
    let device = state.workers.identity(&pair.token).await?;
    assert_eq!(device.capabilities, ["space.scan", WORKSPACE]);
    for other in [session("test", "other"), session("other", "owner")] {
        assert!(
            state
                .workers
                .enqueue(&other, submit(&device.id, WORKSPACE, input.clone()))
                .await
                .is_err()
        );
    }
    for invalid in [
        json!({}),
        json!({"action":"shell"}),
        json!({"action":"run","jobs":[]}),
        json!({"action":"run","jobs":[{}, {}, {}, {}, {}, {}, {}, {}, {}]}),
        json!({"action":"run","jobs":"bad"}),
        json!({"action":"describe","padding":"x".repeat(32_768)}),
    ] {
        assert!(
            state
                .workers
                .enqueue(&owner, submit(&device.id, WORKSPACE, invalid))
                .await
                .is_err()
        );
    }
    let first = submit(&device.id, WORKSPACE, input.clone());
    let queued = state.workers.enqueue(&owner, first.clone()).await?;
    assert_eq!(state.workers.enqueue(&owner, first.clone()).await?, queued);
    let mut conflict = first;
    conflict.input = json!({"action":"run","jobs":[{}]});
    assert!(state.workers.enqueue(&owner, conflict).await.is_err());
    for (tenant, user) in [("test", "other"), ("other", "owner")] {
        assert!(
            client
                .post(format!("{base}/tasks/{}/cancel", queued.id))
                .header("x-test-user", user)
                .header("x-test-tenant", tenant)
                .send()
                .await?
                .status()
                .is_client_error()
        );
    }
    let cancel_url = format!("{base}/tasks/{}/cancel", queued.id);
    assert_eq!(
        client
            .post(&cancel_url)
            .bearer_auth(&pair.token)
            .send()
            .await?
            .status(),
        401
    );
    let cancelled: Value = client
        .post(&cancel_url)
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(cancelled["data"]["state"], "cancelled");
    assert_eq!(
        serde_json::to_value(state.workers.cancel_task(&owner, &queued.id).await?)?,
        cancelled["data"]
    );
    assert!(
        state
            .workers
            .cancel_task(&owner, "invalid-id")
            .await
            .is_err()
    );
    let running = state
        .workers
        .enqueue(
            &owner,
            submit(&device.id, WORKSPACE, json!({"action":"run","jobs":[{}]})),
        )
        .await?;
    let claimed = state
        .workers
        .claim(&device, &Uuid::new_v4().to_string())
        .await?
        .context("未领取运行任务")?;
    assert_eq!(claimed.id, running.id);
    state.workers.cancel_task(&owner, &running.id).await?;
    let complete = CompleteTask {
        lease: claimed.lease.context("租约缺失")?,
        result: Some(json!({"done":true})),
        error: None,
    };
    assert!(
        state
            .workers
            .complete(&device, &running.id, complete.clone())
            .await
            .is_err()
    );
    assert!(
        state
            .workers
            .heartbeat(&device, Some((&running.id, &complete.lease)))
            .await
            .is_err()
    );
    let cleared: bool = sqlx::query_scalar(
        "SELECT lease IS NULL AND lease_until IS NULL FROM worker_tasks WHERE id=$1",
    )
    .bind(&running.id)
    .fetch_one(&state.store.pool)
    .await?;
    assert!(cleared);
    // 完成、失败和中断等终态的取消均保持现有结果。
    for terminal in ["complete", "failed", "interrupted"] {
        let task = state
            .workers
            .enqueue(&owner, submit(&device.id, WORKSPACE, input.clone()))
            .await?;
        sqlx::query("UPDATE worker_tasks SET state=$2,result=$3,error=$4 WHERE id=$1")
            .bind(&task.id)
            .bind(terminal)
            .bind(json!({"saved":true}))
            .bind("saved error")
            .execute(&state.store.pool)
            .await?;
        let before = state.workers.task(&owner, &task.id).await?;
        assert_eq!(state.workers.cancel_task(&owner, &task.id).await?, before);
    }
    let active = state
        .workers
        .enqueue(&owner, submit(&device.id, WORKSPACE, input.clone()))
        .await?;
    let waiting = state
        .workers
        .enqueue(&owner, submit(&device.id, WORKSPACE, input))
        .await?;
    let unaffected = state
        .workers
        .enqueue(&owner, submit(&device.id, "space.scan", json!({})))
        .await?;
    let claim_id = Uuid::new_v4().to_string();
    let claimed = state
        .workers
        .claim(&device, &claim_id)
        .await?
        .context("未领取待关闭任务")?;
    assert_eq!(claimed.id, active.id);
    assert!(
        state
            .workers
            .claim(&device, &Uuid::new_v4().to_string())
            .await?
            .is_none()
    );
    for _ in 0..2 {
        client
            .post(&access)
            .bearer_auth(&pair.token)
            .json(&json!({"enabled":false}))
            .send()
            .await?
            .error_for_status()?;
    }
    assert_eq!(
        state.workers.identity(&pair.token).await?.capabilities,
        ["space.scan"]
    );
    for id in [&active.id, &waiting.id] {
        let row = sqlx::query(
            "SELECT state,lease,lease_until IS NULL AS cleared FROM worker_tasks WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&state.store.pool)
        .await?;
        assert_eq!(row.try_get::<String, _>("state")?, "cancelled");
        assert!(row.try_get::<Option<String>, _>("lease")?.is_none());
        assert!(row.try_get::<bool, _>("cleared")?);
    }
    assert!(state.workers.claim(&device, &claim_id).await?.is_none());
    assert_eq!(
        state.workers.task(&owner, &unaffected.id).await?.state,
        "queued"
    );
    state.workers.cancel_task(&owner, &unaffected.id).await?;
    assert!(
        state
            .workers
            .enqueue(
                &owner,
                submit(&device.id, WORKSPACE, json!({"action":"describe"}))
            )
            .await
            .is_err()
    );
    client
        .post(&access)
        .bearer_auth(&pair.token)
        .json(&json!({"enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    verify_bridge(state, &device.id).await?;
    state.workers.revoke(&owner, &device.id).await?;
    assert_eq!(
        client
            .post(&access)
            .bearer_auth(&pair.token)
            .json(&json!({"enabled":true}))
            .send()
            .await?
            .status(),
        401
    );
    Ok(())
}

async fn verify_bridge(state: &RuntimeState, worker: &str) -> Result<()> {
    let components = state.components.as_ref().context("缺少组件运行时")?;
    let source = Uuid::new_v4();
    let pool = &state.store.pool;
    sqlx::query("INSERT INTO component_sources(id,git) VALUES($1,$2)")
        .bind(source)
        .bind(format!("https://example.invalid/{source}"))
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO component_versions(digest,source_id,archive,version,source_commit,description,metadata) VALUES('workspace-test',$1,$2,'1.0.0','test','{}','{}')").bind(source).bind(Vec::<u8>::new()).execute(pool).await?;
    sqlx::query("INSERT INTO component_installations(tenant_id,source_id,digest,generation) VALUES('test',$1,'workspace-test',$2)").bind(source).bind(Uuid::new_v4()).execute(pool).await?;
    sqlx::query("INSERT INTO component_process_actors(source_id,tenant_id,user_id) VALUES($1,'test','owner'),($1,'test','disabled')").bind(source).execute(pool).await?;
    let mut gateway = Gateway {
        components: Arc::downgrade(components),
        start: super::super::model::Start {
            source,
            tenant: "test".into(),
            revision: "workspace-test".into(),
        },
        token: "process-token".into(),
        endpoints: vec![],
        http_endpoints: vec![],
        worker_capabilities: vec![WORKSPACE.into()],
        services: vec![],
        client: reqwest::Client::new(),
        quota: Arc::new(tokio::sync::Semaphore::new(4)),
    };
    let mut headers = HeaderMap::new();
    headers.insert("x-aio-token", "process-token".parse()?);
    let owner = session("test", "owner");
    let desktop = state
        .workers
        .pair(PairRequest {
            label: "desktop".into(),
            platform: "darwin".into(),
            capabilities: vec![DESKTOP.into()],
        })
        .await?;
    state.workers.approve(&owner, &desktop.code).await?;
    let other = state
        .workers
        .pair(PairRequest {
            label: "other".into(),
            platform: "linux".into(),
            capabilities: vec![WORKSPACE.into()],
        })
        .await?;
    state
        .workers
        .approve(&session("test", "other"), &other.code)
        .await?;
    let devices = call(&gateway, &headers, bridge("list", "*")).await?;
    assert_eq!(devices.as_array().context("设备列表无效")?.len(), 1);
    assert_eq!(devices[0]["id"], worker);
    gateway.worker_capabilities.push(DESKTOP.into());
    assert_eq!(
        call(&gateway, &headers, bridge("list", "*"))
            .await?
            .as_array()
            .context("设备列表无效")?
            .len(),
        2
    );
    let desktop_only = call(
        &gateway,
        &headers,
        json!({"tenantId":"test","userId":"owner","operation":"list"}),
    )
    .await?;
    assert_eq!(desktop_only.as_array().context("设备列表无效")?.len(), 1);
    assert_eq!(desktop_only[0]["id"], desktop.device_id);
    for invalid in [
        json!({"action":"run","jobs":[]}),
        json!({"action":"other"}),
        json!({"action":"describe","padding":"x".repeat(32_768)}),
    ] {
        let mut request = bridge("submit", WORKSPACE);
        request["requestId"] = json!(Uuid::new_v4());
        request["workerId"] = json!(worker);
        request["input"] = invalid;
        assert!(call(&gateway, &headers, request).await.is_err());
    }
    let mut request = bridge("submit", WORKSPACE);
    request["requestId"] = json!(Uuid::new_v4());
    request["workerId"] = json!(worker);
    request["input"] = json!({"action":"run","jobs":[{}]});
    let task: Task = serde_json::from_value(call(&gateway, &headers, request.clone()).await?)?;
    assert_eq!(
        call(&gateway, &headers, request.clone()).await?,
        serde_json::to_value(&task)?
    );
    for (field, value) in [
        ("tenantId", "other"),
        ("userId", "other"),
        ("userId", "disabled"),
        ("requestId", "invalid-id"),
        ("capability", DESKTOP),
        ("capability", "shell.execute"),
    ] {
        let mut invalid = request.clone();
        invalid[field] = json!(value);
        assert!(call(&gateway, &headers, invalid).await.is_err());
    }
    assert!(
        call(&gateway, &HeaderMap::new(), request.clone())
            .await
            .is_err()
    );
    let original_source = gateway.start.source;
    gateway.start.source = Uuid::new_v4();
    assert!(call(&gateway, &headers, request.clone()).await.is_err());
    gateway.start.source = original_source;
    for operation in ["task", "cancel"] {
        let mut access = bridge(operation, DESKTOP);
        access["taskId"] = json!(task.id);
        assert!(call(&gateway, &headers, access).await.is_err());
        gateway.worker_capabilities = vec![DESKTOP.into()];
        let mut access = bridge(operation, WORKSPACE);
        access["taskId"] = json!(task.id);
        assert!(call(&gateway, &headers, access.clone()).await.is_err());
        gateway.worker_capabilities.push(WORKSPACE.into());
        for (field, value) in [
            ("tenantId", "other"),
            ("userId", "other"),
            ("userId", "disabled"),
            ("taskId", "invalid-id"),
            ("capability", "*"),
        ] {
            let mut invalid = access.clone();
            invalid[field] = json!(value);
            assert!(call(&gateway, &headers, invalid).await.is_err());
        }
        assert_eq!(state.workers.task(&owner, &task.id).await?.state, "queued");
        let value = call(&gateway, &headers, access.clone()).await?;
        assert!(value["lease"].is_null());
        if operation == "cancel" {
            assert_eq!(value["state"], "cancelled");
            assert_eq!(call(&gateway, &headers, access).await?, value);
        }
    }
    sqlx::query("DELETE FROM component_process_actors WHERE source_id=$1 AND user_id='owner'")
        .bind(source)
        .execute(pool)
        .await?;
    assert!(call(&gateway, &headers, bridge("list", "*")).await.is_err());
    Ok(())
}
