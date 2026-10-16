use super::*;
use crate::identity::{IdentityProvider, SessionContext};
use crate::runtime::server::plugins::{
    CapabilityEffect, CapabilityProvider, CapabilityRequest, RuntimeAssembly, RuntimeCapabilities,
    RuntimeCapability,
};
use crate::runtime::server::{components::Components, store::PluginStore};
use anyhow::{Context, ensure};
use az_plugin_enhancement::{Enhancement, Example, Language, Package};
use serde_json::json;
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Identity {
    revoked: AtomicBool,
}

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, _: &HeaderMap) -> Result<Option<SessionContext>> {
        Ok(None)
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, _: &str, user: &str) -> Result<bool> {
        Ok(user == "user")
    }
    async fn session_active(&self, session: &str, _: &str, user: &str) -> Result<bool> {
        Ok(session == "session" && user == "user" && !self.revoked.load(Ordering::SeqCst))
    }
}

fn session(tenant: &str, grants: Vec<String>) -> SessionContext {
    SessionContext {
        session_id: "session".into(),
        tenant_id: tenant.into(),
        user_id: "user".into(),
        account: "user".into(),
        display_name: "User".into(),
        tenant_label: tenant.into(),
        permissions: grants,
    }
}

struct ScopeProvider;
#[async_trait::async_trait]
impl CapabilityProvider for ScopeProvider {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        ["/v1/host/scope", "/v1/host/session"]
            .into_iter()
            .map(|path| RuntimeCapability {
                path: path.into(),
                description: "测试调用身份".into(),
                effect: CapabilityEffect::Read,
                input_schema: schemars::schema_for!(serde_json::Value),
                output_schema: schemars::schema_for!(serde_json::Value),
            })
            .collect()
    }
    async fn invoke(&self, request: &CapabilityRequest) -> Result<serde_json::Value> {
        let scope = request.scope();
        if request.path == "/v1/host/session" {
            let session = request.session()?;
            return Ok(
                json!({"tenant":session.tenant_id,"user":session.user_id,"source":scope.source_id}),
            );
        }
        ensure!(
            scope.context.user_id.is_none(),
            "Sidecar 不能凭空获得用户身份"
        );
        Ok(json!({"tenant":scope.context.tenant_id,"source":scope.source_id}))
    }
}

struct ScopePlugin;
impl az_plugin_core::Plugin<RuntimeAssembly> for ScopePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> az_plugin_core::PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add::<RuntimeCapabilities>()
                .add_value(ScopeProvider)
                .bind::<dyn CapabilityProvider, ScopeProvider>();
            Ok(())
        })
    }
}

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
    let identity = Arc::new(Identity::default());
    let extensions = crate::runtime::server::plugins::assemble(
        Arc::new(crate::configuration::HostConfig {
            remote_capabilities: None,
            database_url: database.clone(),
            public_origin: "http://localhost".into(),
            cache_root: root.path().join("cache"),
            component_storage: None,
            default_plugins: vec![],
            delivery: None,
            development: None,
        }),
        vec![Arc::new(ScopePlugin)],
    )
    .await?;
    let enhancements = Arc::new(
        crate::runtime::server::enhancements::Enhancements::open(
            pool.clone(),
            az_plugin_enhancement::default_runtime()?,
            Some(
                crate::runtime::server::enhancements::CapabilityEnvironment {
                    services: Arc::downgrade(&extensions),
                    identity: identity.clone(),
                },
            ),
        )
        .await?,
    );
    let resource_catalog = dill::Catalog::builder()
        .add::<crate::runtime::server::plugins::PostgresComponentPlugin>()
        .add::<crate::runtime::server::plugins::FileObjectStoragePlugin>()
        .build();
    let components = Components::open(
        pool.clone(),
        &crate::configuration::ComponentStorage {
            database_url: database.clone(),
            root: root.path().into(),
        },
        Arc::new(crate::runtime::server::components::load_keyring(
            &root.path().join("keys.json"),
        )?),
        identity.clone(),
        resource_catalog.weak_ref(),
        enhancements,
        crate::runtime::server::components::tests::default_executors()?,
    )
    .await?;
    let tenant = uuid::Uuid::new_v4().to_string();
    let source = uuid::Uuid::new_v4();
    let path = "/v1/caller/scope";
    let package = Package {
        schema_version: 1,
        enhancements: vec![Enhancement {
            capabilities: vec!["/v1/host/scope".into()],
            calls: vec![],
            path: path.into(),
            language: Language::Rhai,
            source: "fn invoke(input, context) { call_capability(\"/v1/host/scope\", input) }"
                .into(),
            examples: vec![Example {
                capabilities: vec![az_plugin_enhancement::CapabilityExample {
                    path: "/v1/host/scope".into(),
                    input: json!({}),
                    output: json!({"tenant":"validation","source":"validation"}),
                }],
                input: json!({}),
                output: json!({"tenant":"validation","source":"validation"}),
            }],
        }, Enhancement {
            path: "/v1/caller/session".into(), language: Language::TypeScript,
            source: "export function invoke(input, context) { context.user_id = 'forged'; return call_capability('/v1/host/session', input); }".into(),
            calls: vec![], capabilities: vec!["/v1/host/session".into()],
            examples: vec![Example {
                input: json!({}), output: json!({"tenant":"validation","user":"validation","source":"validation"}),
                capabilities: vec![az_plugin_enhancement::CapabilityExample {
                    path: "/v1/host/session".into(), input: json!({}),
                    output: json!({"tenant":"validation","user":"validation","source":"validation"}),
                }],
            }],
        }],
    };
    let enhancement = components
        .enhancements
        .propose(&tenant, "test", package.clone(), async { Ok(()) })
        .await?;
    components
        .enhancements
        .activate(
            &tenant,
            "test",
            crate::runtime::server::enhancements::Activation {
                revision: Some(enhancement.clone()),
                expected_generation: 0,
            },
            false,
            async { Ok(()) },
        )
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
    let make_gateway = |grants, runtime_capabilities| {
        Arc::new(Gateway {
            components: Arc::downgrade(&components),
            start: super::super::model::Start {
                source,
                tenant: tenant.clone(),
                revision: revision.clone(),
            },
            token: "fixture-token".into(),
            endpoints: vec![],
            http_endpoints: vec![],
            worker_capabilities: vec![],
            services: vec![],
            enhancements: grants,
            runtime_capabilities,
            client: reqwest::Client::new(),
            quota: Arc::new(tokio::sync::Semaphore::new(2)),
        })
    };
    let socket = root.path().join("broker.sock");
    let listener = tokio::net::UnixListener::bind(&socket)?;
    let router = super::super::broker::router(make_gateway(
        vec![path.into(), "/v1/caller/session".into()],
        vec!["/v1/host/scope".into(), "/v1/host/session".into()],
    ));
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
        let user_request = json!({"path":"/v1/caller/session","input":{}});
        let invoke_user = || client.post(endpoint).header("x-aio-token", "fixture-token").json(&user_request);
        assert_eq!(invoke_user().send().await?.status(), 403);
        let permissions = vec!["runtime:/v1/host/session".into()];
        let user_session = session(&tenant, permissions.clone());
        let authorization = components.services.enter(&user_session, &source.to_string())?;
        let id = &authorization.context.request_id;
        let output: EnhancementResponse = invoke_user().header("x-aio-context", id).header("x-aio-user-id", "forged").send().await?.error_for_status()?.json().await?;
        assert_eq!(output.output, json!({"tenant":tenant,"user":"user","source":source.to_string()}));
        for grants in [vec![], vec!["*".into(), "plugin:manage".into()]] {
            let unauthorized = components.services.enter(&session(&tenant, grants), &source.to_string())?;
            assert_eq!(invoke_user().header("x-aio-context", &unauthorized.context.request_id).send().await?.status(), 403);
        }
        let foreign_source = components.services.enter(&user_session, "other-plugin")?;
        assert_eq!(invoke_user().header("x-aio-context", &foreign_source.context.request_id).send().await?.status(), 403);
        let foreign_tenant = components.services.enter(&session("other-tenant", permissions), &source.to_string())?;
        assert_eq!(invoke_user().header("x-aio-context", &foreign_tenant.context.request_id).send().await?.status(), 403);
        assert_eq!(invoke_user().header("x-aio-context", "forged").send().await?.status(), 403);
        identity.revoked.store(true, Ordering::SeqCst);
        assert_eq!(invoke_user().header("x-aio-context", id).send().await?.status(), 403);
        identity.revoked.store(false, Ordering::SeqCst);
        let expired = id.clone();
        drop(authorization);
        assert_eq!(invoke_user().header("x-aio-context", expired).send().await?.status(), 403);
        let denied_gateway = make_gateway(vec![path.into()], vec![]);
        let mut headers = HeaderMap::new();
        headers.insert("x-aio-token", "fixture-token".parse()?);
        let denied = super::invoke(State(denied_gateway), headers, Json(EnhancementRequest { path: path.into(), input: json!({}) })).await;
        assert_eq!(denied.status(), 403);
        let mut composed = package.clone();
        let mut leaf = composed.enhancements[0].clone();
        leaf.path = "/v1/caller/identity".into();
        composed.enhancements[0].language = Language::TypeScript;
        composed.enhancements[0].calls = vec![leaf.path.clone()];
        composed.enhancements[0].capabilities.clear();
        composed.enhancements[0].source = "export function invoke(input, context) { context.tenant_id = 'forged'; return call_enhancement('/v1/caller/identity', input); }".into();
        composed.enhancements.push(leaf);
        let updated = components.enhancements.propose(&tenant, "test", composed, async { Ok(()) }).await?;
        components.enhancements.activate(&tenant, "test", crate::runtime::server::enhancements::Activation { revision: Some(updated.clone()), expected_generation: 1 }, false, async { Ok(()) }).await?.context("激活组合增强失败")?;
        // 已启动进程不会因增强增加依赖就自动取得新权限。
        assert_eq!(client.post(endpoint).header("x-aio-token", "fixture-token").json(&request).send().await?.status(), 403);
        let socket = root.path().join("composed-broker.sock");
        let listener = tokio::net::UnixListener::bind(&socket)?;
        let router = super::super::broker::router(make_gateway(vec![path.into(), "/v1/caller/identity".into()], vec!["/v1/host/scope".into()]));
        let composed_server = tokio::spawn(async move { axum::serve(listener, router).await });
        let composed_client = reqwest::Client::builder().unix_socket(socket).no_proxy().build()?;
        let response = async {
            composed_client.post(endpoint).header("x-aio-token", "fixture-token").json(&request).send().await?.error_for_status()?.json::<EnhancementResponse>().await
        }.await;
        composed_server.abort();
        let output = response?;
        assert_eq!(output.revision, updated);
        assert_eq!(output.output, json!({"tenant":tenant,"source":source.to_string()}));
        let denied = json!({"path":"/v1/not/granted","input":{}});
        assert_eq!(client.post(endpoint).header("x-aio-token", "fixture-token").json(&denied).send().await?.status(), 403);
        sqlx::query("UPDATE component_installations SET enabled=false WHERE tenant_id=$1 AND source_id=$2").bind(&tenant).bind(source).execute(&pool).await?;
        assert_eq!(client.post(endpoint).header("x-aio-token", "fixture-token").json(&request).send().await?.status(), 403);
        Ok::<_, anyhow::Error>(())
    }.await;
    server.abort();
    outcome
}
