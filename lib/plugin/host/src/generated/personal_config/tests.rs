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
        Ok(headers
            .get("x-test-user")
            .and_then(|h| h.to_str().ok())
            .map(|user| SessionContext {
                session_id: user.into(),
                user_id: user.into(),
                account: user.into(),
                display_name: user.into(),
                tenant_id: headers
                    .get("x-test-tenant")
                    .and_then(|h| h.to_str().ok())
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
#[test]
fn bash_function_contract_rejects_invalid_names_and_attributes() {
    use super::{model::WriteEntry, util};
    let mut entry = WriteEntry {
        id: uuid::Uuid::new_v4().to_string(),
        expected: None,
        kind: "function".into(),
        target: "open_app".into(),
        layer: "shared".into(),
        format: "bash".into(),
        secret: true,
        executable: false,
        deleted: false,
        content: "printf '%s\\n' hello".into(),
    };
    assert!(util::validate(&entry).is_ok());
    for name in ["2bad", "bad-name", "bad;touch", ""] {
        entry.target = name.into();
        assert!(util::validate(&entry).is_err());
    }
    entry.target = "open_app".into();
    entry.executable = true;
    assert!(util::validate(&entry).is_err());
    entry.executable = false;
    entry.format = "text".into();
    assert!(util::validate(&entry).is_err());
    entry.format = "bash".into();
    entry.content = "x".repeat(32 * 1024 + 1);
    assert!(util::validate(&entry).is_err());
}
#[test]
fn crdt_file_contract_requires_encoded_private_updates() {
    use super::{model::WriteEntry, util};
    let mut entry = WriteEntry {
        id: uuid::Uuid::new_v4().to_string(),
        expected: None,
        kind: "file".into(),
        target: ".add_fn".into(),
        layer: "shared".into(),
        format: "yjs-v1".into(),
        secret: true,
        executable: false,
        deleted: false,
        content: "AAA=".into(),
    };
    assert!(util::validate(&entry).is_ok());
    entry.content = "alias ll='ls'".into();
    assert!(util::validate(&entry).is_err());
    entry.content = "AAA=".into();
    entry.secret = false;
    assert!(util::validate(&entry).is_err());
    entry.secret = true;
    entry.target = "../outside".into();
    assert!(util::validate(&entry).is_err());
}

#[tokio::test]
#[ignore = "需要 AIO_TEST_DATABASE_URL 和 AIO_SPACE_TEST_CLI，使用隔离 schema 和设备目录"]
async fn personal_configuration_devices_isolation_revisions_and_sync() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("personal_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut database = reqwest::Url::parse(&database)?;
    database
        .query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        database.as_str(),
        "http://127.0.0.1:1",
        directory.path(),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let app = crate::generated::worker::controller::router()
        .merge(super::controller::router(state.clone()))
        .with_state(state.clone());
    let server = tokio::spawn(axum::serve(listener, app).into_future());
    let client = reqwest::Client::new();
    let browser = format!("{base}/api/runtime/personal-config");
    let worker = format!("{base}/api/runtime/workers/personal-config");
    let mut pairings = vec![];
    for label in ["MacBook", "Mac mini"] {
        let response: Value = client
            .post(format!("{base}/api/runtime/workers/pairings"))
            .json(&json!({"label":label,"platform":"darwin","capabilities":["space.scan"]}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let pair = response["data"].clone();
        client
            .post(format!(
                "{base}/api/runtime/workers/pairings/{}",
                pair["code"].as_str().context("配对码")?
            ))
            .header("x-test-user", "owner")
            .json(&json!({}))
            .send()
            .await?
            .error_for_status()?;
        let token = pair["token"].as_str().context("设备凭据")?;
        assert_eq!(
            client
                .get(format!("{worker}/catalog"))
                .bearer_auth(token)
                .send()
                .await?
                .status(),
            403
        );
        assert_eq!(
            client
                .put(format!(
                    "{browser}/devices/{}",
                    pair["device_id"].as_str().unwrap()
                ))
                .header("x-test-user", "owner")
                .json(&json!({"enabled":true}))
                .send()
                .await?
                .status(),
            403
        );
        client
            .put(format!("{worker}/self"))
            .bearer_auth(token)
            .json(&json!({"enabled":true}))
            .send()
            .await?
            .error_for_status()?;
        pairings.push(pair);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut input = json!({"id":id,"expected":null,"kind":"file","target":".config/editor.json","layer":"shared","format":"jsonc","secret":true,"executable":false,"deleted":false,"content":"{\"alpha\":1,\"beta\":1}"});
    let entry: Value = client
        .post(format!("{browser}/entries"))
        .header("x-test-user", "owner")
        .json(&input)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    input["expected"] = entry["data"]["revision"].clone();
    for (user, tenant) in [("other", "test"), ("owner", "another")] {
        let result: Value = client
            .get(format!("{browser}/catalog"))
            .header("x-test-user", user)
            .header("x-test-tenant", tenant)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(result["data"]["entries"], json!([]));
        assert_eq!(
            client
                .get(format!("{browser}/entries/{id}"))
                .header("x-test-user", user)
                .header("x-test-tenant", tenant)
                .send()
                .await?
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("{browser}/entries/{id}/history"))
                .header("x-test-user", user)
                .header("x-test-tenant", tenant)
                .send()
                .await?
                .json::<Value>()
                .await?["data"],
            json!([])
        );
    }
    assert_eq!(
        client
            .get(format!("{browser}/catalog"))
            .send()
            .await?
            .status(),
        401
    );
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT ciphertext FROM personal_config_entries WHERE id=$1")
            .bind(&id)
            .fetch_one(&state.store.pool)
            .await?;
    assert!(!bytes.windows(5).any(|v| v == b"alpha"));
    // 两台设备修改相同版本时只接受一次提交；重放相同请求仍然幂等。
    let mut other = input.clone();
    input["content"] = json!("{\"alpha\":2,\"beta\":1}");
    other["content"] = json!("{\"alpha\":1,\"beta\":2}");
    let send = |body: Value, index: usize| {
        client
            .post(format!("{worker}/entries"))
            .bearer_auth(pairings[index]["token"].as_str().unwrap())
            .json(&body)
            .send()
    };
    let (a, b) = tokio::join!(send(input.clone(), 0), send(other.clone(), 1));
    let a = a?;
    let b = b?;
    assert_eq!(
        [a.status().as_u16(), b.status().as_u16()]
            .iter()
            .filter(|&&s| s == 409)
            .count(),
        1
    );
    let winner = if a.status().is_success() {
        input.clone()
    } else {
        other
    };
    send(winner, 0).await?.error_for_status()?;
    let historical: Value = client
        .get(format!(
            "{browser}/entries/{id}?revision={}",
            entry["data"]["revision"]
        ))
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(historical["data"]["content"], "{\"alpha\":1,\"beta\":1}");
    // 用真实 CLI 的两份独立 HOME / 加密状态验证双向合并与覆盖层。
    let cli = std::env::var("AIO_SPACE_TEST_CLI")?;
    let mut homes = vec![];
    let mut profiles = vec![];
    for (index, pair) in pairings.iter().enumerate() {
        let home = directory.path().join(format!("home-{index}"));
        let profile = directory.path().join(format!("profile-{index}"));
        tokio::fs::create_dir(&home).await?;
        tokio::fs::create_dir(&profile).await?;
        let file = profile.join("worker.json");
        tokio::fs::write(&file,serde_json::to_vec(&json!({"origin":base,"token":pair["token"],"deviceId":pair["device_id"],"root":home}))?).await?;
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).await?;
        command(
            &cli,
            &profile,
            &[
                "config-enable",
                "--root",
                home.to_str().unwrap(),
                "--foreground",
            ],
        )
        .await?;
        homes.push(home);
        profiles.push(profile);
    }
    let initial: Value =
        serde_json::from_slice(&tokio::fs::read(homes[0].join(".config/editor.json")).await?)?;
    let mut first = initial.clone();
    first["left"] = json!("book");
    let mut second = initial;
    second["right"] = json!("mini");
    tokio::fs::write(
        homes[0].join(".config/editor.json"),
        serde_json::to_vec(&first)?,
    )
    .await?;
    tokio::fs::write(
        homes[1].join(".config/editor.json"),
        serde_json::to_vec(&second)?,
    )
    .await?;
    for index in [0, 1, 0] {
        let result = command(&cli, &profiles[index], &["config-sync"]).await?;
        assert_eq!(result["phase"], "complete", "{result}");
    }
    let merged: Value =
        serde_json::from_slice(&tokio::fs::read(homes[0].join(".config/editor.json")).await?)?;
    assert_eq!(merged["left"], "book");
    assert_eq!(merged["right"], "mini");
    assert_eq!(
        tokio::fs::read(homes[0].join(".config/editor.json")).await?,
        tokio::fs::read(homes[1].join(".config/editor.json")).await?
    );
    for (index, value) in ["A", "B"].iter().enumerate() {
        let mut content = merged.clone();
        content["left"] = json!(value);
        tokio::fs::write(
            homes[index].join(".config/editor.json"),
            serde_json::to_vec(&content)?,
        )
        .await?;
    }
    command(&cli, &profiles[0], &["config-sync"]).await?;
    let conflict = command(&cli, &profiles[1], &["config-sync"]).await?;
    assert_eq!(conflict["phase"], "conflict");
    let c = &conflict["conflicts"][0];
    client.post(format!("{browser}/resolve")).header("x-test-user","owner").json(&json!({"device":pairings[1]["device_id"],"entry":c["id"],"local":c["local"],"remote":c["remote"],"side":"remote"})).send().await?.error_for_status()?;
    assert_eq!(
        command(&cli, &profiles[1], &["config-sync"]).await?["phase"],
        "complete"
    );
    let device_layer = format!("device:{}", pairings[1]["device_id"].as_str().unwrap());
    command(
        &cli,
        &profiles[0],
        &["env-set", "--name", "EDITOR", "--value", "shared"],
    )
    .await?;
    command(
        &cli,
        &profiles[1],
        &[
            "env-set",
            "--name",
            "EDITOR",
            "--value",
            "mini",
            "--layer",
            &device_layer,
        ],
    )
    .await?;
    for index in [0, 1] {
        command(&cli, &profiles[index], &["config-sync"]).await?;
        let env: Value = serde_json::from_slice(
            &tokio::fs::read(homes[index].join(".config/aio-space/environment.json")).await?,
        )?;
        assert_eq!(env["EDITOR"], if index == 0 { "shared" } else { "mini" });
    }
    command(
        &cli,
        &profiles[0],
        &[
            "function-set",
            "--name",
            "where_aio",
            "--value",
            "printf '%s' '{{aio.home}}'",
        ],
    )
    .await?;
    for index in [0, 1] {
        assert_eq!(
            command(&cli, &profiles[index], &["config-sync"]).await?["phase"],
            "complete"
        );
        let source = homes[index].join(".config/aio-space/functions.bash");
        let output = tokio::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                ". \"$1\"; where_aio",
                "check",
            ])
            .arg(source)
            .output()
            .await?;
        assert!(output.status.success());
        let canonical_home = tokio::fs::canonicalize(&homes[index]).await?;
        assert_eq!(output.stdout, canonical_home.to_string_lossy().as_bytes());
    }
    let function_id: String = sqlx::query_scalar(
        "SELECT id FROM personal_config_entries WHERE tenant_id='test' AND user_id='owner' AND kind='function' AND target='where_aio'",
    )
    .fetch_one(&state.store.pool)
    .await?;
    let ciphertext: Vec<u8> =
        sqlx::query_scalar("SELECT ciphertext FROM personal_config_entries WHERE id=$1")
            .bind(function_id)
            .fetch_one(&state.store.pool)
            .await?;
    assert!(!ciphertext.windows(8).any(|part| part == b"aio.home"));
    let revision: Value = client
        .get(format!("{browser}/catalog"))
        .header("x-test-user", "owner")
        .send()
        .await?
        .json()
        .await?;
    let wait = client
        .get(format!(
            "{worker}/changes?after={}&wait=10",
            revision["data"]["revision"]
        ))
        .bearer_auth(pairings[0]["token"].as_str().unwrap())
        .send();
    let disable = async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        client
            .put(format!(
                "{browser}/devices/{}",
                pairings[0]["device_id"].as_str().unwrap()
            ))
            .header("x-test-user", "owner")
            .json(&json!({"enabled":false}))
            .send()
            .await
    };
    let (wait, disabled) = tokio::join!(wait, disable);
    disabled?.error_for_status()?;
    assert_eq!(wait?.status(), 403);
    // 第二台设备仍独立在线；撤销它后不能继续读取配置。
    client
        .get(format!("{worker}/catalog"))
        .bearer_auth(pairings[1]["token"].as_str().unwrap())
        .send()
        .await?
        .error_for_status()?;
    client
        .delete(format!(
            "{base}/api/runtime/workers/{}",
            pairings[1]["device_id"].as_str().unwrap()
        ))
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(
        client
            .get(format!("{worker}/catalog"))
            .bearer_auth(pairings[1]["token"].as_str().unwrap())
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
async fn command(cli: &str, profile: &std::path::Path, args: &[&str]) -> Result<Value> {
    let output = tokio::process::Command::new("node")
        .arg(cli)
        .args(args)
        .env("AIO_SPACE_CONFIG_DIR", profile)
        .output()
        .await?;
    anyhow::ensure!(
        output.status.success(),
        "CLI 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}
