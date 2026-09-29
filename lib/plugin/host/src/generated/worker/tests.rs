use super::*;
use crate::{
    generated::worker::model::{PairRequest, SubmitTask},
    identity::{IdentityProvider, SessionContext},
    runtime::server::RuntimeState,
};
use anyhow::{Context, Result};
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::sync::Arc;

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn machine_identity_pairing_and_deletion() -> Result<()> {
    use super::model::{PairRequest, SubmitTask};
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
        &isolated.to_string(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
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
        label: "same-name".into(),
        platform: "darwin".into(),
        capabilities: vec!["space.scan".into()],
        machine_id: Some(uuid::Uuid::new_v4().to_string()),
    };
    let first = state.workers.pair(request.clone()).await?;
    state.workers.approve(&session, &first.code).await?;
    let identity = state.workers.identity(&first.token).await?;
    for _ in 0..2 {
        state
            .workers
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
    }
    assert!(
        state
            .workers
            .claim(&identity, &uuid::Uuid::new_v4().to_string())
            .await?
            .is_some()
    );
    let second = state.workers.pair(request.clone()).await?;
    state.workers.approve(&session, &second.code).await?;
    let devices = state.workers.list(&session).await?;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, second.device_id);
    assert!(state.workers.identity(&first.token).await.is_err());
    let cancelled: i64 = sqlx::query_scalar("SELECT count(*) FROM worker_tasks WHERE worker_id=$1 AND state='cancelled' AND lease IS NULL AND lease_until IS NULL")
        .bind(&first.device_id).fetch_one(&state.store.pool).await?;
    assert_eq!(cancelled, 2);

    // 主机名相同、标识不同的机器互不覆盖；没有标识的历史设备也不猜测合并。
    let other = state
        .workers
        .pair(PairRequest {
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
            ..request.clone()
        })
        .await?;
    state.workers.approve(&session, &other.code).await?;
    let legacy = state
        .workers
        .pair(PairRequest {
            machine_id: None,
            ..request.clone()
        })
        .await?;
    state.workers.approve(&session, &legacy.code).await?;
    assert_eq!(state.workers.list(&session).await?.len(), 3);

    // 不同用户使用相同标识不能撤销当前用户的设备。
    let outsider = SessionContext {
        user_id: "outsider".into(),
        ..session.clone()
    };
    let outside = state.workers.pair(request.clone()).await?;
    state.workers.approve(&outsider, &outside.code).await?;
    assert_eq!(state.workers.list(&session).await?.len(), 3);
    assert!(
        state
            .workers
            .revoke(&outsider, &second.device_id)
            .await
            .is_err()
    );

    // 两次并发授权只留下一个同标识设备。
    let third = state.workers.pair(request.clone()).await?;
    let fourth = state.workers.pair(request.clone()).await?;
    let (a, b) = tokio::join!(
        state.workers.approve(&session, &third.code),
        state.workers.approve(&session, &fourth.code)
    );
    a?;
    b?;
    assert_eq!(state.workers.list(&session).await?.len(), 3);
    assert_eq!(state.workers.list(&outsider).await?.len(), 1);

    // 填满 32 台后，替换已有标识仍然成功；新增机器被拒绝。
    sqlx::query("INSERT INTO worker_devices(id,token_hash,label,platform,capabilities,tenant_id,user_id,state) SELECT 'filler-'||n,'filler-'||n,'filler','darwin','[\"space.scan\"]'::jsonb,$1,$2,'active' FROM generate_series(1,29) n")
        .bind(&session.tenant_id).bind(&session.user_id).execute(&state.store.pool).await?;
    let replacement = state.workers.pair(request.clone()).await?;
    state.workers.approve(&session, &replacement.code).await?;
    assert_eq!(state.workers.list(&session).await?.len(), 32);
    let excess = state
        .workers
        .pair(PairRequest {
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
            ..request
        })
        .await?;
    assert!(state.workers.approve(&session, &excess.code).await.is_err());
    state
        .workers
        .revoke(&session, &replacement.device_id)
        .await?;
    state
        .workers
        .revoke(&session, &replacement.device_id)
        .await?;
    assert!(
        !state
            .workers
            .list(&session)
            .await?
            .iter()
            .any(|device| device.id == replacement.device_id)
    );
    assert!(state.workers.identity(&replacement.token).await.is_err());

    state.store.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
}

struct Identity;
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(user) = headers.get("x-test-user").and_then(|v| v.to_str().ok()) else {
            return Ok(None);
        };
        Ok(Some(SessionContext {
            session_id: user.into(),
            user_id: user.into(),
            account: user.into(),
            display_name: user.into(),
            tenant_id: headers
                .get("x-test-tenant")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("test")
                .into(),
            tenant_label: "test".into(),
            permissions: vec![],
        }))
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
async fn post(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Result<reqwest::Response> {
    let request = client
        .post(format!("{base}/api/runtime/workers{path}"))
        .json(&body);
    Ok(match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
    .send()
    .await?)
}
#[tokio::test]
#[ignore = "需要隔离 PostgreSQL、restic 和已构建的空间 worker，设置 AIO_TEST_DATABASE_URL / AIO_SPACE_TEST_CLI"]
async fn worker_pairing_tasks_archives_and_revocation_end_to_end() -> Result<()> {
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
    let database = isolated.to_string();
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        &database,
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let app = controller::router().with_state(state.clone());
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let client = reqwest::Client::new();
    let pair:Value=post(&client,&base,"/pairings",None,json!({"label":"test-worker","platform":"darwin","capabilities":["space.scan","space.archive","space.archive-list","space.archive-restore"]})).await?.error_for_status()?.json().await?;
    let pair = &pair["data"];
    let code = pair["code"].as_str().context("缺少配对码")?;
    let token = pair["token"].as_str().context("缺少设备凭据")?;
    let device = pair["device_id"].as_str().context("缺少设备 ID")?;
    assert_eq!(
        post(
            &client,
            &base,
            "/claim",
            Some(token),
            json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":0})
        )
        .await?
        .status(),
        401
    );
    assert_eq!(
        post(
            &client,
            &base,
            &format!("/pairings/{code}"),
            None,
            json!({})
        )
        .await?
        .status(),
        401
    );
    client
        .post(format!("{base}/api/runtime/workers/pairings/{code}"))
        .header("x-test-user", "owner")
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?;
    assert!(
        client
            .post(format!("{base}/api/runtime/workers/pairings/{code}"))
            .header("x-test-user", "other")
            .json(&json!({}))
            .send()
            .await?
            .status()
            .is_client_error()
    );
    let other: Value = client
        .get(format!("{base}/api/runtime/workers"))
        .header("x-test-user", "other")
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(other["data"], json!([]));
    // 应用控制由设备所属账号显式授权，其他用户或租户不能开启。
    let desktop = format!("{base}/api/runtime/workers/{device}/desktop");
    for (user, tenant) in [("other", "test"), ("owner", "other-tenant")] {
        assert!(
            client
                .put(&desktop)
                .header("x-test-user", user)
                .header("x-test-tenant", tenant)
                .json(&json!({"enabled":true}))
                .send()
                .await?
                .status()
                .is_client_error()
        );
    }
    let app_id = uuid::Uuid::new_v4().to_string();
    let app_task = json!({"id":app_id,"worker_id":device,"capability":"desktop.open-app","input":{"application":"Postman"}});
    let submit = || {
        client
            .post(format!("{base}/api/runtime/workers/tasks"))
            .header("x-test-user", "owner")
            .json(&app_task)
    };
    assert!(submit().send().await?.status().is_client_error());
    client
        .put(&desktop)
        .header("x-test-user", "owner")
        .json(&json!({"enabled":true}))
        .send()
        .await?
        .error_for_status()?;
    for _ in 0..2 {
        submit().send().await?.error_for_status()?;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM worker_tasks WHERE id=$1")
        .bind(&app_id)
        .fetch_one(&state.store.pool)
        .await?;
    assert_eq!(count, 1);
    let claim_id = uuid::Uuid::new_v4().to_string();
    let app_claim: Value = post(
        &client,
        &base,
        "/claim",
        Some(token),
        json!({"request_id":claim_id,"wait_seconds":0}),
    )
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert_eq!(app_claim["data"]["id"], app_id);
    // 模拟领取响应丢失，原请求不能分配第二个任务或更换租约。
    let repeated: Value = post(
        &client,
        &base,
        "/claim",
        Some(token),
        json!({"request_id":claim_id,"wait_seconds":0}),
    )
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert_eq!(repeated["data"], app_claim["data"]);
    let task_url = format!("{base}/api/runtime/workers/tasks/{app_id}");
    assert!(
        client
            .get(&task_url)
            .header("x-test-user", "other")
            .send()
            .await?
            .status()
            .is_client_error()
    );
    let app_read: Value = client
        .get(&task_url)
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(app_read["data"]["lease"].is_null());
    client
        .put(&desktop)
        .header("x-test-user", "owner")
        .json(&json!({"enabled":false}))
        .send()
        .await?
        .error_for_status()?;
    let cancelled: Value = client
        .get(&task_url)
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(cancelled["data"]["state"], "cancelled");
    assert!(
        post(
            &client,
            &base,
            &format!("/tasks/{app_id}/complete"),
            Some(token),
            json!({"lease":app_claim["data"]["lease"],"result":{"running":true}})
        )
        .await?
        .status()
        .is_client_error()
    );
    let input = root.path().join("input");
    tokio::fs::create_dir(&input).await?;
    tokio::fs::write(input.join("record.txt"), "worker archive round trip 中文").await?;
    let profile = root.path().join("profile");
    tokio::fs::create_dir(&profile).await?;
    let profile_file = profile.join("worker.json");
    tokio::fs::write(
        &profile_file,
        serde_json::to_vec(
            &json!({"origin":base,"token":token,"deviceId":device,"root":root.path()}),
        )?,
    )
    .await?;
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(&profile_file, std::fs::Permissions::from_mode(0o600)).await?;
    let cli = std::env::var("AIO_SPACE_TEST_CLI")?;
    for capability in ["space.scan", "space.archive", "space.archive-list"] {
        let id = uuid::Uuid::new_v4().to_string();
        let task = json!({"id":id,"worker_id":device,"capability":capability,"input":{"path":input,"depth":0}});
        assert!(
            client
                .post(format!("{base}/api/runtime/workers/tasks"))
                .header("x-test-user", "other")
                .json(&task)
                .send()
                .await?
                .status()
                .is_client_error()
        );
        client
            .post(format!("{base}/api/runtime/workers/tasks"))
            .header("x-test-user", "owner")
            .json(&task)
            .send()
            .await?
            .error_for_status()?;
        let output = tokio::process::Command::new("node")
            .args([&cli, "worker", "--once"])
            .env("AIO_SPACE_CONFIG_DIR", &profile)
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "worker 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let tasks: Value = client
            .get(format!("{base}/api/runtime/workers/tasks"))
            .header("x-test-user", "owner")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let completed = tasks["data"]
            .as_array()
            .context("任务列表无效")?
            .iter()
            .find(|v| v["id"] == id)
            .context("未找到任务")?;
        assert_eq!(completed["state"], "complete", "{completed}");
        assert!(completed["lease"].is_null());
    }
    let secret: Value = client
        .get(format!("{base}/api/runtime/workers/archive-config"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let password = secret["data"]["password"]
        .as_str()
        .context("缺少自动归档密钥")?;
    let encrypted: Vec<u8> =
        sqlx::query_scalar("SELECT ciphertext FROM worker_vaults WHERE user_id='owner'")
            .fetch_one(&state.store.pool)
            .await?;
    assert!(
        !encrypted
            .windows(password.len())
            .any(|v| v == password.as_bytes())
    );
    let restored = root.path().join("restored");
    let status = tokio::process::Command::new("restic")
        .args([
            "--repo",
            &format!("rest:{base}/api/runtime/workers/archive/"),
            "--no-cache",
            "restore",
            "latest",
            "--target",
            restored.to_str().context("路径无效")?,
            "--verify",
        ])
        .env("RESTIC_PASSWORD", password)
        .env("RESTIC_REST_USERNAME", "worker")
        .env("RESTIC_REST_PASSWORD", token)
        .output()
        .await?;
    anyhow::ensure!(
        status.status.success(),
        "恢复失败：{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let original = std::fs::canonicalize(input.join("record.txt"))?;
    let restored_file = restored.join(original.strip_prefix("/")?);
    assert_eq!(std::fs::read(&original)?, std::fs::read(restored_file)?);
    let id = uuid::Uuid::new_v4().to_string();
    client
        .post(format!("{base}/api/runtime/workers/tasks"))
        .header("x-test-user", "owner")
        .json(&json!({"id":id,"worker_id":device,"capability":"space.scan","input":{}}))
        .send()
        .await?
        .error_for_status()?;
    // 已结束的旧领取请求不能领取后来排队的任务。
    let old: Value = post(
        &client,
        &base,
        "/claim",
        Some(token),
        json!({"request_id":claim_id,"wait_seconds":0}),
    )
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert!(old["data"].is_null());
    let (a, b) = tokio::join!(
        post(
            &client,
            &base,
            "/claim",
            Some(token),
            json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":0})
        ),
        post(
            &client,
            &base,
            "/claim",
            Some(token),
            json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":0})
        )
    );
    let a: Value = a?.json().await?;
    let b: Value = b?.json().await?;
    assert_ne!(a["data"].is_null(), b["data"].is_null());
    let claimed = if a["data"].is_null() {
        &b["data"]
    } else {
        &a["data"]
    };
    assert!(
        post(
            &client,
            &base,
            &format!("/tasks/{id}/complete"),
            Some(token),
            json!({"lease":"wrong","result":{}})
        )
        .await?
        .status()
        .is_client_error()
    );
    sqlx::query("UPDATE worker_tasks SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(&id)
        .execute(&state.store.pool)
        .await?;
    assert!(
        post(
            &client,
            &base,
            &format!("/tasks/{id}/complete"),
            Some(token),
            json!({"lease":claimed["lease"],"result":{}})
        )
        .await?
        .status()
        .is_client_error()
    );
    // 空闲长轮询等待完整窗口；下一次等待中入队任务，最多一个轮询周期内返回。
    let start = std::time::Instant::now();
    let empty: Value = post(
        &client,
        &base,
        "/claim",
        Some(token),
        json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":2}),
    )
    .await?
    .error_for_status()?
    .json()
    .await?;
    assert!(empty["data"].is_null());
    assert!(start.elapsed() >= std::time::Duration::from_secs(2));
    let pending_id = uuid::Uuid::new_v4().to_string();
    let wait = post(
        &client,
        &base,
        "/claim",
        Some(token),
        json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":10}),
    );
    let enqueue = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        client
            .post(format!("{base}/api/runtime/workers/tasks"))
            .header("x-test-user", "owner")
            .json(&json!({"id":pending_id,"worker_id":device,"capability":"space.scan","input":{}}))
            .send()
            .await?
            .error_for_status()?;
        Ok::<(), anyhow::Error>(())
    };
    let start = std::time::Instant::now();
    let (received, queued) = tokio::join!(wait, enqueue);
    queued?;
    let received: Value = received?.error_for_status()?.json().await?;
    assert_eq!(received["data"]["id"], pending_id);
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
    let completion = json!({"lease":received["data"]["lease"],"result":{"completed":true}});
    for _ in 0..2 {
        post(
            &client,
            &base,
            &format!("/tasks/{pending_id}/complete"),
            Some(token),
            completion.clone(),
        )
        .await?
        .error_for_status()?;
    }
    // 长轮询中撤销权限必须终止等待，不把离线误作注销。
    let waiting = post(
        &client,
        &base,
        "/claim",
        Some(token),
        json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":10}),
    );
    let revoking = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        client
            .delete(format!("{base}/api/runtime/workers/{device}"))
            .header("x-test-user", "owner")
            .send()
            .await?
            .error_for_status()?;
        Ok::<(), anyhow::Error>(())
    };
    let (response, revoked) = tokio::join!(waiting, revoking);
    revoked?;
    assert_eq!(response?.status(), 401);
    client
        .delete(format!("{base}/api/runtime/workers/{device}"))
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(
        post(
            &client,
            &base,
            "/claim",
            Some(token),
            json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":0})
        )
        .await?
        .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{base}/api/runtime/workers/archive/config"))
            .bearer_auth(token)
            .send()
            .await?
            .status(),
        401
    );
    server.abort();
    state.store.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
}

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
        &isolated.to_string(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
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
    let first = state.workers.pair(request.clone()).await?;
    state.workers.approve(&session, &first.code).await?;
    state
        .workers
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
    let second = state.workers.pair(request.clone()).await?;
    state.workers.approve(&session, &second.code).await?;
    let devices = state.workers.list(&session).await?;
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
        .fetch_one(&state.store.pool)
        .await?;
    assert_eq!(stale_state, "revoked");
    let task_state: String =
        sqlx::query_scalar("SELECT state FROM worker_tasks WHERE worker_id=$1")
            .bind(&first.device_id)
            .fetch_one(&state.store.pool)
            .await?;
    assert_eq!(task_state, "cancelled");

    let other = state
        .workers
        .pair(PairRequest {
            // 故意与第一台同名同平台，但 machine_id 不同，必须各自保留。
            label: "same-machine".into(),
            platform: "darwin".into(),
            capabilities: vec!["space.scan".into()],
            machine_id: Some("machine-bbb".into()),
        })
        .await?;
    state.workers.approve(&session, &other.code).await?;
    let devices = state.workers.list(&session).await?;
    assert_eq!(
        devices
            .iter()
            .filter(|device| device.status != "revoked")
            .count(),
        2
    );

    state.store.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
}
