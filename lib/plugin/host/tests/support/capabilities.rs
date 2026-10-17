use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use anyhow::Result;
use az_plugin_contract::{CapabilityGrants, InvocationScope, RequestContext};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_host::{
    configuration::HostConfig,
    identity::{IdentityProvider, SessionContext},
    runtime::server::plugins::{
        CapabilityEffect, CapabilityProvider, CapabilityRequest, RuntimeAssembly, RuntimeCapability,
    },
};
use serde_json::{Value, json};

pub const PATH: &str = "/v1/native/execute";

#[derive(Default)]
pub struct Identity {
    pub revoked: AtomicBool,
    pub removed: AtomicBool,
    pub runtime_revoked: AtomicBool,
    pub additional_paths: Vec<String>,
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
        Ok(Some(SessionContext {
            session_id: "session".into(),
            user_id: "user".into(),
            account: "user".into(),
            display_name: "User".into(),
            tenant_id: if actor == "other" { "other" } else { "test" }.into(),
            tenant_label: "Test".into(),
            permissions: match actor {
                "capable" | "other" if !self.runtime_revoked.load(Ordering::SeqCst) => {
                    std::iter::once(PATH.to_owned())
                        .chain(self.additional_paths.iter().cloned())
                        .map(|path| format!("runtime:{path}"))
                        .chain((actor == "other").then(|| "plugin:manage".into()))
                        .collect()
                }
                "manager" => vec!["plugin:manage".into()],
                _ => vec![],
            },
        }))
    }
    async fn member_active(&self, _: &str, user: &str) -> Result<bool> {
        Ok(user == "user" && !self.removed.load(Ordering::SeqCst))
    }
    async fn session_active(&self, session: &str, _: &str, user: &str) -> Result<bool> {
        Ok(session == "session" && user == "user" && !self.revoked.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl az_plugin_host::authorization::AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
}

pub fn scope() -> InvocationScope {
    InvocationScope {
        source_id: "native-test".into(),
        revision: "test".into(),
        context: RequestContext {
            tenant_id: Some("test".into()),
            user_id: Some("user".into()),
            session_id: Some("session".into()),
            request_id: "request".into(),
        },
        grants: CapabilityGrants {
            runtime: vec![PATH.into()],
            ..Default::default()
        },
    }
}

pub fn config(root: &std::path::Path) -> HostConfig {
    HostConfig {
        remote_capabilities: None,
        database_url: "no database in native capability host".into(),
        public_origin: "http://127.0.0.1".into(),
        cache_root: root.join("cache"),
        component_storage: None,
        default_plugins: vec![],
        delivery: None,
        development: None,
    }
}

#[derive(Default)]
pub struct Probe {
    pub calls: AtomicUsize,
    pub active: AtomicUsize,
    pub entered: tokio::sync::Notify,
    pub scopes: Mutex<Vec<InvocationScope>>,
}

struct Active(Arc<Probe>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Provider(pub Arc<Probe>, pub i64);

#[async_trait::async_trait]
impl CapabilityProvider for Provider {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        vec![RuntimeCapability {
            path: PATH.into(),
            description: "异步原生测试能力".into(),
            effect: CapabilityEffect::Write,
            input_schema: serde_json::Map::from_iter([(
                "type".into(),
                json!(["integer", "string", "null"]),
            )])
            .into(),
            output_schema: schemars::schema_for!(i64),
        }]
    }
    fn available(&self, _: &str, tenant: &str) -> bool {
        tenant == "test"
    }
    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value> {
        assert!(Instant::now() < request.deadline());
        assert_eq!(request.session()?.tenant_id, "test");
        self.0
            .scopes
            .lock()
            .map_err(|_| anyhow::anyhow!("范围记录锁损坏"))?
            .push(request.scope().clone());
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(self.0.clone());
        self.0.entered.notify_one();
        match request.input.as_str() {
            Some("wait") => std::future::pending().await,
            Some("fail") => anyhow::bail!("测试上游失败"),
            Some("invalid") => Ok(json!("invalid output")),
            Some("large") => Ok(json!("x".repeat(65536))),
            _ => Ok(json!(self.1)),
        }
    }
}

pub struct ProviderPlugin(pub Arc<Probe>, pub i64);
impl Plugin<RuntimeAssembly> for ProviderPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Provider(self.0.clone(), self.1))
                .bind::<dyn CapabilityProvider, Provider>();
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    let provider = state.service::<Provider>()?;
                    anyhow::ensure!(
                        provider.0.active.load(Ordering::SeqCst) == 0,
                        "释放资源前必须结束所有能力调用"
                    );
                    Ok(())
                })
            });
            Ok(())
        })
    }
}
