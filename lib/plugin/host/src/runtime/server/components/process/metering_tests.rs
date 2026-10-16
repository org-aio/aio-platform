use crate::metering::{MeterOutcome, MeteringProvider};
use anyhow::Result;
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, PartialEq, Eq)]
struct Usage {
    tenant: String,
    user: String,
    source: String,
    resource: String,
    quantity: i64,
    key: String,
}

#[derive(Default)]
pub(super) struct Meter {
    calls: Mutex<Vec<Usage>>,
    fail: AtomicBool,
}

#[async_trait::async_trait]
impl MeteringProvider for Meter {
    async fn meter(
        &self,
        tenant: &str,
        user: &str,
        source: &str,
        resource: &str,
        quantity: i64,
        key: &str,
    ) -> Result<MeterOutcome> {
        anyhow::ensure!(!self.fail.load(Ordering::SeqCst), "测试计费后端失败");
        self.calls
            .lock()
            .map_err(|_| anyhow::anyhow!("测试锁损坏"))?
            .push(Usage {
                tenant: tenant.into(),
                user: user.into(),
                source: source.into(),
                resource: resource.into(),
                quantity,
                key: key.into(),
            });
        Ok(MeterOutcome {
            amount_micros: 12,
            grant_consumed: 0,
            balance_charged_micros: 12,
            balance_after_micros: 88,
            duplicate: false,
        })
    }
}

pub(super) async fn verify(
    client: &reqwest::Client,
    pool: &sqlx::PgPool,
    tenant: &str,
    source: uuid::Uuid,
    meter: &Arc<Meter>,
    installed: bool,
    revoked: &AtomicBool,
) -> Result<()> {
    let endpoint = "http://localhost/meter";
    let input = json!({"tenantId": tenant, "userId": "user", "sourceId": "provider:model", "resource": "tokens", "quantity": 3, "idempotencyKey": "meter-test"});
    let send = |input: serde_json::Value| {
        client
            .post(endpoint)
            .header("x-aio-token", "fixture-token")
            .json(&input)
            .send()
    };
    assert_eq!(send(input.clone()).await?.status(), 403);
    sqlx::query("INSERT INTO component_process_actors(source_id,tenant_id,user_id) VALUES($1,$2,'user') ON CONFLICT DO NOTHING").bind(source).bind(tenant).execute(pool).await?;
    let response = send(input.clone()).await?;
    if installed {
        let body = response
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        assert_eq!(body["status"], 200);
        assert_eq!(body["body"]["amount_micros"], 12);
    } else {
        assert_eq!(response.status(), 204);
    }
    let mut forged = input.clone();
    forged["tenantId"] = json!("another-tenant");
    assert_eq!(send(forged).await?.status(), 403);
    assert_eq!(
        client.post(endpoint).json(&input).send().await?.status(),
        403
    );
    revoked.store(true, Ordering::SeqCst);
    assert_eq!(send(input.clone()).await?.status(), 403);
    revoked.store(false, Ordering::SeqCst);
    if installed {
        meter.fail.store(true, Ordering::SeqCst);
        assert_eq!(send(input).await?.status(), 403);
    }
    let calls = meter
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("测试锁损坏"))?;
    if installed {
        assert_eq!(
            *calls,
            [Usage {
                tenant: tenant.into(),
                user: "user".into(),
                source: "provider:model".into(),
                resource: "tokens".into(),
                quantity: 3,
                key: "meter-test".into(),
            }]
        );
    } else {
        assert!(calls.is_empty());
    }
    Ok(())
}
