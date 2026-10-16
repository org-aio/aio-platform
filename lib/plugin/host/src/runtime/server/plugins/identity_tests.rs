use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use anyhow::{Result, ensure};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};

use super::{RuntimeAssembly, tests::config};
use crate::{
    authorization::AuthorizationProvider,
    identity::{IdentityProvider, SessionContext},
    metering::{MeterOutcome, MeteringProvider},
    runtime::server::{RuntimeState, http_error::RuntimeError, request_context},
};

#[derive(Default)]
struct Identity<const N: u8> {
    revoked: AtomicBool,
    started: AtomicUsize,
    stopped: AtomicUsize,
    fail: bool,
}

#[async_trait::async_trait]
impl<const N: u8> IdentityProvider for Identity<N> {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        Ok((headers.get("authorization").and_then(|h| h.to_str().ok())
            == Some("Bearer identity-test"))
        .then(|| SessionContext {
            session_id: "session".into(),
            user_id: "user".into(),
            account: "account".into(),
            display_name: "User".into(),
            tenant_id: "tenant".into(),
            tenant_label: "Tenant".into(),
            permissions: vec!["plugin:manage".into()],
        }))
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == "tenant" && user == "user" && !self.revoked.load(Ordering::SeqCst))
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "session" && self.member_active(tenant, user).await?)
    }
}

#[derive(Default)]
struct Policy<const N: u8>(AtomicBool);

#[async_trait::async_trait]
impl<const N: u8> AuthorizationProvider for Policy<N> {
    async fn can_publish(&self, session: &SessionContext) -> Result<bool> {
        ensure!(
            session.tenant_id == "tenant" && session.user_id == "user",
            "授权策略收到错误身份"
        );
        Ok(self.0.load(Ordering::SeqCst))
    }
}

struct Meter<const N: u8>;
#[async_trait::async_trait]
impl<const N: u8> MeteringProvider for Meter<N> {
    async fn meter(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: i64,
        _: &str,
    ) -> Result<MeterOutcome> {
        anyhow::bail!("该测试不能执行计费")
    }
}

#[derive(Default)]
struct Providers<const N: u8> {
    identity: Option<Arc<Identity<N>>>,
    policy: Option<Arc<Policy<N>>>,
    meter: bool,
}

impl<const N: u8> Plugin<RuntimeAssembly> for Providers<N> {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            if let Some(identity) = &self.identity {
                assembly
                    .services
                    .add_builder(identity.clone())
                    .bind::<dyn IdentityProvider, Identity<N>>();
                assembly.on_initialize::<Self>(|state, _| {
                    Box::pin(async move {
                        let identity = state.service::<Identity<N>>()?;
                        identity.started.fetch_add(1, Ordering::SeqCst);
                        ensure!(!identity.fail, "身份提供者初始化失败");
                        Ok(())
                    })
                });
                assembly.on_stop::<Self>(|state, _| {
                    Box::pin(async move {
                        state
                            .service::<Identity<N>>()?
                            .stopped
                            .fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                });
            }
            if let Some(policy) = &self.policy {
                assembly
                    .services
                    .add_builder(policy.clone())
                    .bind::<dyn AuthorizationProvider, Policy<N>>();
            }
            if self.meter {
                assembly
                    .services
                    .add_value(Meter::<N>)
                    .bind::<dyn MeteringProvider, Meter<N>>();
            }
            Ok(())
        })
    }
}

struct ProtectedRoutes;
impl Plugin<RuntimeAssembly> for ProtectedRoutes {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn IdentityProvider>();
            assembly.require_service::<dyn AuthorizationProvider>();
            assembly.add_routes(|_| {
                Ok(Router::new()
                    .route("/private", get(read))
                    .route("/publish", get(publish)))
            });
            Ok(())
        })
    }
}

async fn read(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<SessionContext>, RuntimeError> {
    Ok(Json(request_context::authenticate(&state, &headers).await?))
}
async fn publish(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<SessionContext>, RuntimeError> {
    Ok(Json(
        request_context::authenticate_publish_manager(&state, &headers).await?,
    ))
}

#[tokio::test]
async fn independent_identity_and_policy_plugins_share_live_instances() -> Result<()> {
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity::<1>::default());
    let policy = Arc::new(Policy::<2>::default());
    let state = RuntimeState::initialize(
        config(root.path()),
        vec![
            Arc::new(ProtectedRoutes),
            Arc::new(Providers::<1> {
                identity: Some(identity.clone()),
                ..Default::default()
            }),
            Arc::new(Providers::<2> {
                policy: Some(policy.clone()),
                ..Default::default()
            }),
        ],
    )
    .await?;
    let identity_service: Arc<dyn IdentityProvider> = identity.clone();
    let policy_service: Arc<dyn AuthorizationProvider> = policy.clone();
    assert!(Arc::ptr_eq(
        &state.service::<dyn IdentityProvider>()?,
        &identity_service
    ));
    assert!(Arc::ptr_eq(
        &state.service::<dyn AuthorizationProvider>()?,
        &policy_service
    ));
    assert!(state.service::<dyn MeteringProvider>().is_err());
    assert!(!root.path().join("cache").exists());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let router = crate::runtime::server::router(state.clone())?;
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::builder().no_proxy().build()?;
    let result = async {
        assert_eq!(
            client.get(format!("{base}/private")).send().await?.status(),
            401
        );
        let session = client
            .get(format!("{base}/private"))
            .bearer_auth("identity-test")
            .send()
            .await?
            .error_for_status()?
            .json::<SessionContext>()
            .await?;
        assert_eq!(session.tenant_id, "tenant");
        assert_eq!(
            client
                .get(format!("{base}/publish"))
                .bearer_auth("identity-test")
                .send()
                .await?
                .status(),
            403
        );
        policy.0.store(true, Ordering::SeqCst);
        assert_eq!(
            client
                .get(format!("{base}/publish"))
                .bearer_auth("identity-test")
                .send()
                .await?
                .status(),
            200
        );
        identity.revoked.store(true, Ordering::SeqCst);
        assert_eq!(
            client
                .get(format!("{base}/private"))
                .bearer_auth("identity-test")
                .send()
                .await?
                .status(),
            401
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    server.abort();
    let _ = server.await;
    state.shutdown().await?;
    assert_eq!(identity.started.load(Ordering::SeqCst), 1);
    assert_eq!(identity.stopped.load(Ordering::SeqCst), 1);
    result
}

#[tokio::test]
async fn missing_or_duplicate_security_providers_fail_before_initialization() -> Result<()> {
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity::<1>::default());
    let cases: Vec<Vec<DynPlugin<RuntimeAssembly>>> = vec![
        vec![Arc::new(ProtectedRoutes)],
        vec![
            Arc::new(ProtectedRoutes),
            Arc::new(Providers::<1> {
                identity: Some(identity.clone()),
                ..Default::default()
            }),
        ],
        vec![
            Arc::new(ProtectedRoutes),
            Arc::new(Providers::<1> {
                policy: Some(Arc::default()),
                ..Default::default()
            }),
        ],
        vec![
            Arc::new(Providers::<1> {
                identity: Some(identity.clone()),
                ..Default::default()
            }),
            Arc::new(Providers::<2> {
                identity: Some(Arc::default()),
                ..Default::default()
            }),
        ],
        vec![
            Arc::new(Providers::<1> {
                identity: Some(identity.clone()),
                policy: Some(Arc::default()),
                ..Default::default()
            }),
            Arc::new(Providers::<2> {
                policy: Some(Arc::default()),
                ..Default::default()
            }),
        ],
        vec![
            Arc::new(Providers::<1> {
                identity: Some(identity.clone()),
                meter: true,
                ..Default::default()
            }),
            Arc::new(Providers::<2> {
                meter: true,
                ..Default::default()
            }),
        ],
    ];
    for plugins in cases {
        assert!(
            RuntimeState::initialize(config(root.path()), plugins)
                .await
                .is_err()
        );
        assert_eq!(identity.started.load(Ordering::SeqCst), 0);
        assert_eq!(identity.stopped.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[tokio::test]
async fn failed_identity_initialization_runs_its_cleanup() -> Result<()> {
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity::<1> {
        fail: true,
        ..Default::default()
    });
    let result = RuntimeState::initialize(
        config(root.path()),
        vec![Arc::new(Providers::<1> {
            identity: Some(identity.clone()),
            ..Default::default()
        })],
    )
    .await;
    assert!(result.is_err());
    assert_eq!(identity.started.load(Ordering::SeqCst), 1);
    assert_eq!(identity.stopped.load(Ordering::SeqCst), 1);
    Ok(())
}

struct TransientIdentity;
impl Plugin<RuntimeAssembly> for TransientIdentity {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(|| Arc::new(Identity::<1>::default()))
                .bind::<dyn IdentityProvider, Identity<1>>();
            Ok(())
        })
    }
}

#[tokio::test]
async fn transient_identity_cannot_split_state_between_protocols() -> Result<()> {
    let root = tempfile::tempdir()?;
    let error = RuntimeState::initialize(config(root.path()), vec![Arc::new(TransientIdentity)])
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("未拒绝瞬态身份提供者"))?;
    assert!(error.to_string().contains("Dill 单例"));
    Ok(())
}
