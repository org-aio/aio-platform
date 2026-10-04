use crate::{
    identity::{IdentityProvider, SessionContext},
    runtime::server::RuntimeState,
};
use anyhow::{Context, Result};
use axum::http::HeaderMap;
use base64::Engine;
use serde_json::{Value, json};

fn b64(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes.as_ref())
}
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

/// 通道开通、设备鉴权、二进制往返、文本类型、分页和跨用户隔离。
#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn clipboard_channel_roundtrip_isolation_and_revocation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("clipboard_test_{}", uuid::Uuid::new_v4().simple());
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
    let browser = format!("{base}/api/runtime/clipboard");
    let worker = format!("{base}/api/runtime/workers/clipboard");

    let response: Value = client
        .post(format!("{base}/api/runtime/workers/pairings"))
        .json(&json!({"label":"Mac mini","platform":"darwin","capabilities":["space.scan"]}))
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

    // 未开通通道时设备读写被拒绝，网页仍可读写自己的剪切板。
    assert_eq!(
        client
            .get(format!("{worker}/head"))
            .bearer_auth(token)
            .send()
            .await?
            .status(),
        403
    );
    let written: Value = client
        .post(format!("{browser}/items"))
        .header("x-test-user", "owner")
        .json(&json!({"kind":"text","mime":"text/plain","data":b64("你好")}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = written["data"]["id"]
        .as_str()
        .context("条目 ID")?
        .to_string();

    // 只有本机设备凭据可以开通自己的通道。
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

    // 设备读取文本条目，正文与写入一致。
    let read: Value = client
        .get(format!("{worker}/items/{id}"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(read["data"]["data"], b64("你好"));
    assert_eq!(read["data"]["item"]["kind"], "text");

    // 二进制（图片）按字节往返，不经过 UTF-8 转码。
    let blob: Vec<u8> = (0u8..=255).collect();
    let encoded = b64(&blob);
    let image: Value = client
        .post(format!("{worker}/items"))
        .bearer_auth(token)
        .json(&json!({"kind":"image","mime":"image/png","name":"shot.png","data":encoded}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let image_id = image["data"]["id"].as_str().context("图片 ID")?;
    let read: Value = client
        .get(format!("{worker}/items/{image_id}"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(read["data"]["data"], encoded);
    assert_eq!(read["data"]["item"]["name"], "shot.png");

    // 显式投递把条目的元数据和正文作为任务发给设备，设备领取后可本机应用。
    let pushed: Value = client
        .post(format!("{browser}/items/{image_id}/push"))
        .header("x-test-user", "owner")
        .json(&json!({"devices":[]}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let task_id = pushed["data"][0].as_str().context("投递任务")?;
    let claim: Value = client
        .post(format!("{base}/api/runtime/workers/claim"))
        .bearer_auth(token)
        .json(&json!({"request_id":uuid::Uuid::new_v4().to_string(),"wait_seconds":0}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(claim["data"]["id"], *task_id);
    assert_eq!(claim["data"]["capability"], "clipboard.sync");
    assert_eq!(claim["data"]["input"]["id"], *image_id);
    // 任务只携带条目 ID，设备凭自己的通道拉取正文，因此图片不受任务输入配额限制。
    assert_eq!(claim["data"]["input"].as_object().unwrap().len(), 1);
    let pulled: Value = client
        .get(format!("{worker}/items/{image_id}"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(pulled["data"]["data"], encoded);
    let lease = claim["data"]["lease"].as_str().context("租约")?;
    client
        .post(format!(
            "{base}/api/runtime/workers/tasks/{task_id}/complete"
        ))
        .bearer_auth(token)
        .json(&json!({"lease":lease,"result":{"applied":true}}))
        .send()
        .await?
        .error_for_status()?;

    // 非 UTF-8 文本被拒绝；分页返回最新优先。
    assert_eq!(
        client
            .post(format!("{browser}/items"))
            .header("x-test-user", "owner")
            .json(&json!({"kind":"text","mime":"text/plain","data":b64([0xff,0xfe])}))
            .send()
            .await?
            .status(),
        400
    );
    let page: Value = client
        .get(format!("{browser}/items?limit=1"))
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(page["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"]["items"][0]["id"], *image_id);
    assert!(page["data"]["cursor"].as_i64().unwrap() > 0);

    // 超过保留上限后最旧条目及其分片被淘汰，读取返回 404。
    let first_id = id.clone();
    for index in 0..70 {
        client
            .post(format!("{browser}/items"))
            .header("x-test-user", "owner")
            .json(&json!({"kind":"text","mime":"text/plain","data":b64(format!("entry-{index}"))}))
            .send()
            .await?
            .error_for_status()?;
    }
    assert_eq!(
        client
            .get(format!("{browser}/items/{first_id}"))
            .header("x-test-user", "owner")
            .send()
            .await?
            .status(),
        404
    );

    // 其他用户看不到任何条目。
    let empty: Value = client
        .get(format!("{browser}/head"))
        .header("x-test-user", "intruder")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(empty["data"]["revision"], 0);

    // 撤销配对后设备请求立即失效。
    client
        .delete(format!(
            "{base}/api/runtime/workers/{}",
            pair["device_id"].as_str().unwrap()
        ))
        .header("x-test-user", "owner")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(
        client
            .get(format!("{worker}/head"))
            .bearer_auth(token)
            .send()
            .await?
            .status(),
        401
    );

    server.abort();
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    Ok(())
}
