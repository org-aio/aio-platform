use super::*;
use crate::{
    identity::{IdentityProvider, SessionContext},
    runtime::server::RuntimeState,
};
use anyhow::{Context, Result};
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::sync::Arc;

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
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn device_label_is_owner_scoped_trimmed_and_persistent() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("worker_label_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut isolated = reqwest::Url::parse(&database)?;
    isolated
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let root = tempfile::tempdir()?;
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        &isolated.to_string(),
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let session = SessionContext {
        session_id: "owner".into(),
        user_id: "owner".into(),
        account: "owner".into(),
        display_name: "Owner".into(),
        tenant_id: "test".into(),
        tenant_label: "Test".into(),
        permissions: vec![],
    };
    let pairing = state
        .workers
        .pair(PairRequest {
            label: "host-name".into(),
            platform: "darwin".into(),
            capabilities: vec!["space.scan".into()],
        })
        .await?;
    state.workers.approve(&session, &pairing.code).await?;
    let renamed = state
        .workers
        .update_label(
            &session,
            &pairing.device_id,
            UpdateLabelRequest {
                label: "  工作笔记本  ".into(),
            },
        )
        .await?;
    assert_eq!(renamed.label, "工作笔记本");
    assert_eq!(state.workers.list(&session).await?[0].label, "工作笔记本");
    let stored: String = sqlx::query_scalar("SELECT label FROM worker_devices WHERE id=$1")
        .bind(&pairing.device_id)
        .fetch_one(&state.store.pool)
        .await?;
    assert_eq!(stored, "工作笔记本");
    for invalid in ["", "   ", "标".repeat(121)] {
        assert!(
            state
                .workers
                .update_label(
                    &session,
                    &pairing.device_id,
                    UpdateLabelRequest {
                        label: invalid.into()
                    }
                )
                .await
                .is_err()
        );
    }
    let other = SessionContext {
        user_id: "other".into(),
        account: "other".into(),
        ..session.clone()
    };
    assert!(
        state
            .workers
            .update_label(
                &other,
                &pairing.device_id,
                UpdateLabelRequest {
                    label: "越权".into()
                }
            )
            .await
            .is_err()
    );
    let other_tenant = SessionContext {
        tenant_id: "other-tenant".into(),
        ..session
    };
    assert!(
        state
            .workers
            .update_label(
                &other_tenant,
                &pairing.device_id,
                UpdateLabelRequest {
                    label: "越权".into()
                }
            )
            .await
            .is_err()
    );
    state.store.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
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
