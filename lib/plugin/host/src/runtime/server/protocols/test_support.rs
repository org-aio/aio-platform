use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, ensure};
use az_plugin_enhancement::{Enhancement, Example, Language, Package};
use rmcp::{
    RoleClient, ServiceExt,
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};

use crate::{
    identity::{IdentityProvider, SessionContext},
    runtime::server::RuntimeState,
};

pub(super) const PATH: &str = "/v1/math/double";

pub(super) struct Identity {
    pub tenant: String,
    pub revoked: AtomicBool,
}

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<SessionContext>> {
        let Some(actor) = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return Ok(None);
        };
        let permissions = match actor {
            "manager" | "other" => vec!["plugin:manage".into()],
            "slow" => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                vec!["plugin:manage".into()]
            }
            "limited" => vec![format!("enhancement:{PATH}")],
            "composed" => [PATH, "/v1/math/base", "/v1/math/pipeline"]
                .into_iter()
                .map(|path| format!("enhancement:{path}"))
                .collect(),
            "denied" => vec![],
            _ => return Ok(None),
        };
        Ok(Some(SessionContext {
            session_id: actor.into(),
            user_id: actor.into(),
            account: actor.into(),
            display_name: actor.into(),
            tenant_id: if actor == "other" {
                format!("{}-other", self.tenant)
            } else {
                self.tenant.clone()
            },
            tenant_label: "Test".into(),
            permissions,
        }))
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn session_active(&self, session: &str, _: &str, _: &str) -> Result<bool> {
        Ok(!matches!(session, "limited" | "composed") || !self.revoked.load(Ordering::SeqCst))
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
}

pub(super) struct Fixture {
    pub base: String,
    pub state: RuntimeState,
    pub identity: Arc<Identity>,
    pub server: tokio::task::JoinHandle<std::io::Result<()>>,
    _root: tempfile::TempDir,
}

impl Fixture {
    pub async fn check_omitted_plugins(&self) -> Result<()> {
        let mut state = self.state.clone();
        state.extensions =
            crate::runtime::server::plugins::assemble(state.config.clone(), Vec::new()).await?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let router = crate::runtime::server::router(state);
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let result = async {
            for path in ["/api/runtime/mcp", "/aio.runtime.v1.Enhancements/List"] {
                assert_eq!(
                    reqwest::Client::new()
                        .post(format!("{origin}{path}"))
                        .send()
                        .await?
                        .status(),
                    404
                );
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        server.abort();
        result
    }

    pub async fn open() -> Result<Self> {
        let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
        ensure!(
            database.contains("127.0.0.1") && database.contains("_test"),
            "只接受本机独立测试库"
        );
        let root = tempfile::tempdir()?;
        let identity = Arc::new(Identity {
            tenant: uuid::Uuid::new_v4().to_string(),
            revoked: AtomicBool::new(false),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        use crate::runtime::server::plugins::*;
        // 协议宿主只装配增强依赖；无组件存储、密钥、Wasm 引擎和 Sidecar 资源。
        let state = RuntimeState::initialize_with_plugins(
            crate::configuration::HostConfig {
                database_url: database,
                public_origin: base.clone(),
                cache_root: root.path().join("cache"),
                component_storage: None,
                default_plugins: vec![],
                delivery: None,
                development: None,
            },
            identity.clone(),
            vec![
                Arc::new(McpPlugin),
                Arc::new(GrpcPlugin),
                Arc::new(EnhancementsPlugin),
                Arc::new(EnhancementServicePlugin),
                Arc::new(ScriptRuntimePlugin),
                Arc::new(RhaiPlugin),
                Arc::new(TypeScriptPlugin),
                Arc::new(DatabasePlugin),
                Arc::new(PostgresPlugin),
            ],
        )
        .await?;
        assert!(state.optional_components()?.is_none());
        assert!(state.service::<az_plugin_runtime::Keyring>().is_err());
        assert!(state.wasm().is_err());
        assert!(state.process().is_err());
        assert!(state.store().is_err());
        assert!(!root.path().join("cache").exists());
        let router = crate::runtime::server::router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            base,
            state,
            identity,
            server,
            _root: root,
        })
    }

    pub async fn publish(&self, package: Package, generation: i64) -> Result<String> {
        let client = reqwest::Client::new();
        let response: Value = client
            .post(format!("{}/api/runtime/enhancements/revisions", self.base))
            .bearer_auth("manager")
            .json(&package)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let revision = response["revision"]
            .as_str()
            .context("缺少增强版本")?
            .to_owned();
        client
            .put(format!("{}/api/runtime/enhancements/active", self.base))
            .bearer_auth("manager")
            .json(&json!({"revision": revision, "expected_generation":generation}))
            .send()
            .await?
            .error_for_status()?;
        Ok(revision)
    }
}

pub(super) fn package(language: Language) -> Package {
    let source = match language {
        Language::Rhai => "fn invoke(input, context) { input * 2 }",
        Language::TypeScript => {
            "export function invoke(input: number): number { return input * 2; }"
        }
    };
    Package { schema_version: 1, enhancements: vec![
        Enhancement { calls: vec![], path: PATH.into(), language, source: source.into(), examples: vec![Example { input: json!(3), output: json!(6) }] },
        Enhancement { calls: vec![], path: "/v1/caller/scope".into(), language: Language::Rhai, source: "fn invoke(input, context) { #{tenant:context.tenant_id,source:context.source_id} }".into(), examples: vec![Example { input: json!({}), output: json!({"tenant":"validation","source":"validation"}) }] }
    ] }
}

pub(super) async fn mcp_client(base: &str, actor: &str) -> Result<RunningService<RoleClient, ()>> {
    let config = StreamableHttpClientTransportConfig::with_uri(format!("{base}/api/runtime/mcp"))
        .auth_header(actor);
    let transport = StreamableHttpClientTransport::with_client(reqwest_mcp::Client::new(), config);
    Ok(().serve(transport).await?)
}

pub(super) fn auth<T>(message: T, actor: &str) -> Result<tonic::Request<T>> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {actor}").parse()?);
    Ok(request)
}
