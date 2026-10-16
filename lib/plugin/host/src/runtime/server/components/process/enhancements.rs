use anyhow::Result;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use az_plugin_contract::{
    EnhancementRequest, EnhancementResponse, InvocationScope, RequestContext,
};
use std::sync::Arc;

use super::model::Gateway;

/// Sidecar 复用活动版本票据和当前安装包能力，不能在请求中伪造调用归属。
pub(super) async fn invoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<EnhancementRequest>,
) -> Response {
    let result: Result<EnhancementResponse> = async {
        let _permit = gateway.quota.try_acquire()?;
        let components = super::broker::active(&gateway, &headers).await?;
        let scope = InvocationScope {
            source_id: gateway.start.source.to_string(),
            revision: gateway.start.revision.clone(),
            context: RequestContext {
                tenant_id: Some(gateway.start.tenant.clone()),
                ..Default::default()
            },
            // 固定为创建此进程时的能力，避免并发升级让旧票据取得新版额外权限。
            grants: az_plugin_contract::CapabilityGrants {
                enhancements: gateway.enhancements.clone(),
                ..Default::default()
            },
        };
        components.enhancements.invoke(&scope, request).await
    }
    .await;
    match result {
        Ok(response) => Json(response).into_response(),
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"增强能力不可用或调用未授权"})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::server::{
        components::{Components, tests::TestIdentity},
        store::PluginStore,
    };
    use anyhow::{Context, ensure};
    use az_plugin_enhancement::{Enhancement, Example, Language, Package};
    use serde_json::json;
    use sqlx::PgPool;

    #[tokio::test]
    #[ignore = "需要独立本机 PostgreSQL"]
    async fn sidecar_enhancements_bind_identity_grants_and_active_revision() -> Result<()> {
        let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
        ensure!(
            database.contains("127.0.0.1") && database.contains("_test"),
            "只接受本机独立增强测试库"
        );
        let pool = PgPool::connect(&database).await?;
        PluginStore::new(pool.clone()).migrate().await?;
        let root = tempfile::tempdir()?;
        let enhancements = Arc::new(
            crate::runtime::server::enhancements::Enhancements::open(
                pool.clone(),
                az_plugin_enhancement::default_runtime()?,
            )
            .await?,
        );
        let components = Components::open(
            pool.clone(),
            &crate::configuration::ComponentStorage {
                database_url: database.clone(),
                root: root.path().into(),
            },
            Arc::new(crate::runtime::server::components::load_keyring(
                &root.path().join("keys.json"),
            )?),
            Arc::new(TestIdentity),
            dill::Catalog::builder().build().weak_ref(),
            enhancements,
        )
        .await?;
        let tenant = uuid::Uuid::new_v4().to_string();
        let source = uuid::Uuid::new_v4();
        let path = "/v1/caller/scope";
        let package = Package { schema_version: 1, enhancements: vec![Enhancement {
            path: path.into(), language: Language::Rhai,
            source: "fn invoke(input, context) { #{tenant:context.tenant_id,source:context.source_id} }".into(),
            examples: vec![Example { input: json!({}), output: json!({"tenant":"validation","source":"validation"}) }],
        }] };
        let enhancement = components
            .enhancements
            .propose(&tenant, "test", package)
            .await?;
        components
            .enhancements
            .activate(&tenant, "test", Some(&enhancement), 0, false)
            .await?
            .context("激活增强失败")?;
        let revision = format!("{}{}", source.simple(), source.simple());
        // 此夹具只验证 broker 边界，不启动或伪装为已经执行的外部程序。
        sqlx::query("INSERT INTO component_sources(id,git) VALUES($1,$2)")
            .bind(source)
            .bind(format!("https://example.com/{source}.git"))
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO component_versions(digest,source_id,archive,version,source_commit,description,metadata,permissions) VALUES($1,$2,$3,'1.0.0',$4,$5,'{}','{}')")
            .bind(&revision).bind(source).bind(Vec::<u8>::new()).bind("a".repeat(40)).bind(json!({"label":"Fixture","pages":[]})).execute(&pool).await?;
        sqlx::query("INSERT INTO component_installations(tenant_id,source_id,digest,generation) VALUES($1,$2,$3,$4)")
            .bind(&tenant).bind(source).bind(&revision).bind(uuid::Uuid::new_v4()).execute(&pool).await?;
        let gateway = Arc::new(Gateway {
            components: Arc::downgrade(&components),
            start: super::super::model::Start {
                source,
                tenant: tenant.clone(),
                revision,
            },
            token: "fixture-token".into(),
            endpoints: vec![],
            http_endpoints: vec![],
            worker_capabilities: vec![],
            services: vec![],
            enhancements: vec![path.into()],
            client: reqwest::Client::new(),
            quota: Arc::new(tokio::sync::Semaphore::new(2)),
        });
        let socket = root.path().join("broker.sock");
        let listener = tokio::net::UnixListener::bind(&socket)?;
        let router = super::super::broker::router(gateway);
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = reqwest::Client::builder()
            .unix_socket(socket)
            .no_proxy()
            .build()?;
        let endpoint = "http://localhost/enhancements/invoke";
        let request = json!({"path":path,"input":{}});
        let outcome = async {
            assert_eq!(client.post(endpoint).json(&request).send().await?.status(), 403);
            let output: EnhancementResponse = client.post(endpoint).header("x-aio-token", "fixture-token").json(&request).send().await?.error_for_status()?.json().await?;
            assert_eq!(output.output, json!({"tenant":tenant,"source":source.to_string()}));
            assert_eq!(output.revision, enhancement);
            let denied = json!({"path":"/v1/not/granted","input":{}});
            assert_eq!(client.post(endpoint).header("x-aio-token", "fixture-token").json(&denied).send().await?.status(), 403);
            sqlx::query("UPDATE component_installations SET enabled=false WHERE tenant_id=$1 AND source_id=$2").bind(&tenant).bind(source).execute(&pool).await?;
            assert_eq!(client.post(endpoint).header("x-aio-token", "fixture-token").json(&request).send().await?.status(), 403);
            Ok::<_, anyhow::Error>(())
        }.await;
        server.abort();
        outcome
    }
}
