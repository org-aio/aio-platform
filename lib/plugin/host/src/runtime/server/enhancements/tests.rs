use anyhow::{Context, Result, ensure};
use az_plugin_contract::{CapabilityGrants, EnhancementRequest, InvocationScope, RequestContext};
use az_plugin_enhancement::{Enhancement, Example, Language, Package};
use serde_json::json;
use sqlx::PgPool;

use super::Enhancements;

fn package(language: Language, source: &str) -> Package {
    Package {
        schema_version: 1,
        enhancements: vec![Enhancement {
            path: "/v1/math/double".into(),
            language,
            source: source.into(),
            examples: vec![Example {
                input: json!(3),
                output: json!(6),
            }],
        }],
    }
}

fn scope(tenant: &str) -> InvocationScope {
    InvocationScope {
        source_id: "fixture".into(),
        revision: "plugin-version".into(),
        context: RequestContext {
            tenant_id: Some(tenant.into()),
            ..Default::default()
        },
        grants: CapabilityGrants {
            enhancements: vec!["/v1/math/double".into()],
            ..Default::default()
        },
    }
}

fn request() -> EnhancementRequest {
    EnhancementRequest {
        path: "/v1/math/double".into(),
        input: json!(7),
    }
}

fn database() -> Result<String> {
    let url = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    ensure!(
        (url.contains("localhost") || url.contains("127.0.0.1")) && url.contains("_test"),
        "只接受本机独立增强测试库"
    );
    Ok(url)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn enhancements_survive_restart_reject_stale_publish_and_rollback() -> Result<()> {
    let pool = PgPool::connect(&database()?).await?;
    let runtime = Enhancements::open(pool.clone()).await?;
    let tenant = uuid::Uuid::new_v4().to_string();
    let actor = "test-publisher";
    let first = runtime
        .propose(
            &tenant,
            actor,
            package(Language::Rhai, "fn invoke(input, context) { input * 2 }"),
        )
        .await?;
    assert!(runtime.head(&tenant).await?.revision.is_none());
    assert!(runtime.invoke(&scope(&tenant), request()).await.is_err());
    runtime
        .activate(&tenant, actor, Some(&first), 0, false)
        .await?
        .context("首次激活失败")?;
    assert_eq!(
        runtime.invoke(&scope(&tenant), request()).await?.output,
        json!(14)
    );
    let mut ungranted = scope(&tenant);
    ungranted.grants.enhancements.clear();
    assert!(runtime.invoke(&ungranted, request()).await.is_err());
    assert!(
        runtime
            .invoke(&scope("other-tenant"), request())
            .await
            .is_err()
    );
    assert!(
        runtime
            .activate("other-tenant", actor, Some(&first), 0, false)
            .await
            .is_err()
    );
    let mut bad = package(Language::Rhai, "fn invoke(input, context) { input * 3 }");
    assert!(runtime.propose(&tenant, actor, bad.clone()).await.is_err());
    bad.enhancements[0].examples[0].output = json!(9);
    let bad = runtime.propose(&tenant, actor, bad).await?;
    assert!(
        runtime
            .activate(&tenant, actor, Some(&bad), 1, false)
            .await
            .is_err()
    );
    assert!(
        runtime
            .activate(&tenant, actor, Some(&bad), 1, true)
            .await
            .is_err()
    );
    assert_eq!(
        runtime.head(&tenant).await?.revision.as_deref(),
        Some(first.as_str())
    );
    let second = runtime
        .propose(
            &tenant,
            actor,
            package(
                Language::TypeScript,
                "export function invoke(input: number) { return input * 2; }",
            ),
        )
        .await?;
    let (left, right) = tokio::join!(
        runtime.activate(&tenant, actor, Some(&second), 1, false),
        runtime.activate(&tenant, actor, Some(&second), 1, false)
    );
    assert_eq!(
        usize::from(left?.is_some()) + usize::from(right?.is_some()),
        1
    );
    let restored = Enhancements::open(pool).await?;
    let response = restored.invoke(&scope(&tenant), request()).await?;
    assert_eq!(response.revision, second);
    assert_eq!(response.output, json!(14));
    restored
        .activate(&tenant, actor, Some(&first), 2, true)
        .await?
        .context("回滚失败")?;
    assert_eq!(
        runtime.invoke(&scope(&tenant), request()).await?.revision,
        first
    );
    assert!(
        runtime
            .activate(&tenant, actor, Some(&second), 1, false)
            .await?
            .is_none()
    );
    restored
        .activate(&tenant, actor, None, 3, false)
        .await?
        .context("停用失败")?;
    assert!(runtime.invoke(&scope(&tenant), request()).await.is_err());
    Ok(())
}

struct Identity;

#[async_trait::async_trait]
impl crate::identity::IdentityProvider for Identity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<crate::identity::SessionContext>> {
        let Some(actor) = headers
            .get("x-test-actor")
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(None);
        };
        Ok(Some(crate::identity::SessionContext {
            session_id: "test-session".into(),
            user_id: actor.into(),
            account: actor.into(),
            display_name: actor.into(),
            tenant_id: "http-enhancement-test".into(),
            tenant_label: "Test".into(),
            permissions: if actor == "manager" {
                vec!["plugin:manage".into()]
            } else {
                vec![]
            },
        }))
    }
    async fn can_publish(&self, _: &crate::identity::SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn enhancement_http_requires_manager_and_reports_generation_conflicts() -> Result<()> {
    use crate::runtime::server::{RuntimeState, components::Components};
    use std::sync::Arc;
    let database = database()?;
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity);
    let mut state = RuntimeState::isolated_admin_test(
        identity.clone(),
        &database,
        "http://127.0.0.1",
        root.path(),
    )
    .await?;
    state.components = Some(
        Components::open(
            state.store.pool.clone(),
            &database,
            &root.path().join("keyring.json"),
            root.path().join("objects"),
            identity,
        )
        .await?,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/api/runtime/enhancements", listener.local_addr()?);
    let router = super::router().with_state(state);
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::new();
    let outcome = async {
        assert_eq!(client.get(&base).send().await?.status(), 401);
        assert_eq!(
            client
                .get(&base)
                .header("x-test-actor", "reader")
                .send()
                .await?
                .status(),
            403
        );
        let head: serde_json::Value = client
            .get(&base)
            .header("x-test-actor", "manager")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let generation = head["generation"].as_i64().context("缺少活动代数")?;
        let candidate: serde_json::Value = client
            .post(format!("{base}/revisions"))
            .header("x-test-actor", "manager")
            .json(&package(
                Language::Rhai,
                "fn invoke(input, context) { input * 2 }",
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let activation = json!({"revision":candidate["revision"],"expected_generation":generation});
        assert_eq!(
            client
                .put(format!("{base}/active"))
                .header("x-test-actor", "manager")
                .json(&activation)
                .send()
                .await?
                .status(),
            200
        );
        assert_eq!(
            client
                .put(format!("{base}/active"))
                .header("x-test-actor", "manager")
                .json(&activation)
                .send()
                .await?
                .status(),
            409
        );
        let mut foreign = activation;
        foreign["tenant_id"] = json!("other-tenant");
        assert_eq!(
            client
                .put(format!("{base}/active"))
                .header("x-test-actor", "manager")
                .json(&foreign)
                .send()
                .await?
                .status(),
            422
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    server.abort();
    outcome
}
