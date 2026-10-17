#![cfg(any(feature = "mcp", feature = "grpc"))]

#[path = "support/identity.rs"]
mod identity_support;
#[path = "support/capabilities.rs"]
mod support;

use std::{
    any::TypeId,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::{Context, Result};
use az_plugin_core::DynPlugin;
use az_plugin_host::runtime::server::{RuntimeState, plugins::*};
use serde_json::{Value, json};
use support::*;

struct Fixture {
    host: RuntimeHost,
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
    identity: Arc<Identity>,
    probe: Arc<Probe>,
    base: String,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn open(identity: Identity, extra: Vec<DynPlugin<RuntimeAssembly>>) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let identity = Arc::new(identity);
        let probe = Arc::new(Probe::default());
        let mut plugins: Vec<DynPlugin<RuntimeAssembly>> = vec![
            Arc::new(CapabilityServicePlugin),
            Arc::new(ProviderPlugin(probe.clone(), 42)),
        ];
        #[cfg(feature = "mcp")]
        plugins.push(Arc::new(McpPlugin));
        #[cfg(feature = "grpc")]
        plugins.push(Arc::new(GrpcPlugin));
        plugins.extend(extra);
        let plugins = identity_support::with_identity(identity.clone(), plugins);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let mut configuration = config(root.path());
        configuration.public_origin = base.clone();
        let host =
            RuntimeHost::initialize_with_router(configuration, plugins.clone(), |state, router| {
                assert!(state.service::<ScriptRuntime>().is_err());
                #[cfg(feature = "postgres")]
                assert!(state.service::<sqlx::PgPool>().is_err());
                assert!(state.service::<az_plugin_runtime::Keyring>().is_err());
                Ok(router)
            })
            .await?;
        assert!(!root.path().join("cache").exists());
        let router = host.router();
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            host,
            plugins,
            identity,
            probe,
            base,
            server,
            _root: root,
        })
    }

    async fn close(&self) -> Result<()> {
        self.host.shutdown().await?;
        self.server.abort();
        Ok(())
    }

    fn assert_scope(&self) -> Result<()> {
        let scopes = self
            .probe
            .scopes
            .lock()
            .map_err(|_| anyhow::anyhow!("范围记录锁损坏"))?;
        let actual = scopes.last().context("缺少实际调用")?;
        assert_eq!(actual.source_id, "user:user");
        assert_eq!(actual.context.tenant_id, scope().context.tenant_id);
        assert_eq!(actual.context.user_id.as_deref(), Some("user"));
        assert_eq!(actual.context.session_id.as_deref(), Some("session"));
        assert_eq!(actual.grants.runtime, [PATH]);
        assert!(actual.grants.enhancements.is_empty());
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn protocol_only_defaults_do_not_install_script_services_and_identity_is_required()
-> Result<()> {
    #[cfg(feature = "postgres")]
    {
        let plugins = default_plugins()?;
        if !cfg!(any(feature = "rhai", feature = "typescript")) {
            assert!(
                !plugins
                    .iter()
                    .any(|plugin| plugin.as_ref().type_id() == TypeId::of::<ScriptRuntimePlugin>())
            );
            assert!(!plugins.iter().any(
                |plugin| plugin.as_ref().type_id() == TypeId::of::<EnhancementServicePlugin>()
            ));
        }
    }
    let root = tempfile::tempdir()?;
    let mut anonymous: Vec<DynPlugin<RuntimeAssembly>> = vec![Arc::new(CapabilityServicePlugin)];
    #[cfg(feature = "mcp")]
    anonymous.push(Arc::new(McpPlugin));
    #[cfg(feature = "grpc")]
    anonymous.push(Arc::new(GrpcPlugin));
    assert!(
        RuntimeState::initialize(config(root.path()), anonymous)
            .await
            .is_err()
    );
    Ok(())
}

#[cfg(feature = "grpc")]
#[path = "capabilities/grpc.rs"]
mod grpc;
#[cfg(feature = "mcp")]
#[path = "capabilities/mcp.rs"]
mod mcp;
