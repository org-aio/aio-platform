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
    pub runtime_allowed: AtomicBool,
    pub management_allowed: AtomicBool,
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
        let mut permissions = match actor {
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
            "capable" => [
                "enhancement:/v1/workflow/root",
                "enhancement:/v1/workflow/worker",
                "runtime:/v1/workers/list",
                "runtime:/v1/workers/tasks/get",
                "runtime:/v1/workers/tasks/submit",
                "runtime:/v1/workers/tasks/cancel",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            "remote" => [
                "enhancement:/v1/remote/double",
                "runtime:/v1/remote/mcp",
                "runtime:/v1/remote/grpc",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            "evolution" | "evolution-reader" => {
                let mut permissions = vec![
                    "enhancement:/v1/evolution/bootstrap".into(),
                    "enhancement:/v1/evolution/observe".into(),
                    "enhancement:/v1/evolution/control".into(),
                    format!("enhancement:{PATH}"),
                    "runtime:/v1/model/enhancement/source".into(),
                ];
                if actor == "evolution" && self.management_allowed.load(Ordering::SeqCst) {
                    permissions.push("plugin:manage".into());
                }
                for operation in [
                    "head", "package", "catalog", "propose", "activate", "rollback", "history",
                    "observe",
                ] {
                    permissions.push(format!("runtime:/v1/runtime/enhancements/{operation}"));
                }
                permissions
            }
            _ => return Ok(None),
        };
        if !self.runtime_allowed.load(Ordering::SeqCst) {
            permissions.retain(|permission| !permission.starts_with("runtime:"));
        }
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
        Ok(!matches!(
            session,
            "limited" | "composed" | "capable" | "remote" | "evolution"
        ) || !self.revoked.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl crate::authorization::AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn session_has_permissions(
        &self,
        session: &str,
        tenant: &str,
        user: &str,
        permissions: &[String],
    ) -> Result<bool> {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", format!("Bearer {session}").parse()?);
        Ok(self.authenticate(&headers).await?.is_some_and(|current| {
            current.tenant_id == tenant
                && current.user_id == user
                && permissions
                    .iter()
                    .all(|permission| current.permissions.contains(permission))
        }))
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
        let router = crate::runtime::server::router(state)?;
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
        Self::with_plugins(vec![]).await
    }

    pub async fn with_plugins(
        extra: Vec<az_plugin_core::DynPlugin<crate::runtime::server::plugins::RuntimeAssembly>>,
    ) -> Result<Self> {
        Self::configured(extra, None).await
    }

    pub async fn configured(
        extra: Vec<az_plugin_core::DynPlugin<crate::runtime::server::plugins::RuntimeAssembly>>,
        imports: Option<Vec<Value>>,
    ) -> Result<Self> {
        let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
        ensure!(
            database.contains("127.0.0.1") && database.contains("_test"),
            "只接受本机独立测试库"
        );
        let root = tempfile::tempdir()?;
        let identity = Arc::new(Identity {
            tenant: uuid::Uuid::new_v4().to_string(),
            revoked: AtomicBool::new(false),
            runtime_allowed: AtomicBool::new(true),
            management_allowed: AtomicBool::new(true),
        });
        let remote_capabilities = if let Some(mut imports) = imports {
            for import in &mut imports {
                if import["tenant_id"] == "test" {
                    import["tenant_id"] = json!(identity.tenant);
                }
            }
            let path = root.path().join("remote.json");
            std::fs::write(
                &path,
                serde_json::to_vec(&json!({"schema_version":1,"imports":imports}))?,
            )?;
            Some(path)
        } else {
            None
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        use crate::runtime::server::plugins::*;
        let minimal = extra.is_empty();
        let mut plugins: Vec<az_plugin_core::DynPlugin<RuntimeAssembly>> = vec![
            Arc::new(McpPlugin),
            Arc::new(GrpcPlugin),
            Arc::new(EnhancementsPlugin),
            Arc::new(EnhancementServicePlugin),
            Arc::new(CapabilityServicePlugin),
            Arc::new(CapabilitiesPlugin),
            Arc::new(ScriptRuntimePlugin),
            Arc::new(RhaiPlugin),
            Arc::new(TypeScriptPlugin),
            Arc::new(DatabasePlugin),
            Arc::new(PostgresPlugin),
        ];
        plugins.extend(extra);
        // 协议宿主只装配增强依赖；无组件存储、密钥、Wasm 引擎和 Sidecar 资源。
        let state = RuntimeState::initialize(
            crate::configuration::HostConfig {
                remote_capabilities,
                database_url: database,
                public_origin: base.clone(),
                cache_root: root.path().join("cache"),
                component_storage: None,
                default_plugins: vec![],
                delivery: None,
                development: None,
            },
            crate::runtime::server::admin_test_support::with_identity(identity.clone(), plugins),
        )
        .await?;
        assert!(
            state
                .optional_service::<crate::runtime::server::components::Components>()?
                .is_none()
        );
        assert!(state.service::<az_plugin_runtime::Keyring>().is_err());
        assert!(state.wasm().is_err());
        assert!(state.process().is_err());
        if minimal {
            assert!(state.store().is_err());
        }
        assert!(!root.path().join("cache").exists());
        let router = crate::runtime::server::router(state.clone())?;
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
        publish(&self.base, package, generation).await
    }
}

pub(super) async fn publish(base: &str, package: Package, generation: i64) -> Result<String> {
    let client = reqwest::Client::new();
    let response: Value = client
        .post(format!("{}/api/runtime/enhancements/revisions", base))
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
        .put(format!("{}/api/runtime/enhancements/active", base))
        .bearer_auth("manager")
        .json(&json!({"revision": revision, "expected_generation":generation}))
        .send()
        .await?
        .error_for_status()?;
    Ok(revision)
}
pub(super) fn package(language: Language) -> Package {
    let source = match language {
        Language::Rhai => "fn invoke(input, context) { input * 2 }",
        Language::TypeScript => {
            "export function invoke(input: number): number { return input * 2; }"
        }
    };
    Package { schema_version: 1, enhancements: vec![
        Enhancement { input_schema: None, output_schema: None, capabilities: vec![], calls: vec![], path: PATH.into(), language, source: source.into(), examples: vec![Example { capabilities: vec![], input: json!(3), output: json!(6) }] },
        Enhancement { input_schema: None, output_schema: None, capabilities: vec![], calls: vec![], path: "/v1/caller/scope".into(), language: Language::Rhai, source: "fn invoke(input, context) { #{tenant:context.tenant_id,source:context.source_id} }".into(), examples: vec![Example { capabilities: vec![], input: json!({}), output: json!({"tenant":"validation","source":"validation"}) }] }
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
