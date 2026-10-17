//! 正式 Wasm 导入及真实 Sidecar broker socket 共用管理权限；监督器仅替换业务进程。

use super::*;
use crate as az_plugin_host;
include!("../../../../tests/support/identity.rs");
use crate::{
    authorization::AuthorizationProvider,
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, plugins::*},
};
use anyhow::{Context, Result, ensure};
use axum::{Router, routing::post};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{InvocationScope, process::Configuration};
use az_plugin_runtime::HostServices;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

struct Identity {
    revoked: AtomicBool,
}

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, _: &axum::http::HeaderMap) -> Result<Option<SessionContext>> {
        Ok(None)
    }
    async fn member_active(&self, _: &str, user: &str) -> Result<bool> {
        Ok(user == "manager")
    }
    async fn session_active(&self, session: &str, _: &str, user: &str) -> Result<bool> {
        Ok(session == "interactive" && user == "manager")
    }
}

#[async_trait::async_trait]
impl AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(true)
    }
    async fn session_has_permissions(
        &self,
        session: &str,
        tenant: &str,
        user: &str,
        required: &[String],
    ) -> Result<bool> {
        Ok(!self.revoked.load(Ordering::SeqCst)
            && required == ["plugin:manage"]
            && self.session_active(session, tenant, user).await?)
    }
}

fn session(tenant: &str) -> SessionContext {
    SessionContext {
        session_id: "interactive".into(),
        user_id: "manager".into(),
        account: "manager".into(),
        display_name: "Manager".into(),
        tenant_id: tenant.into(),
        tenant_label: "Test".into(),
        permissions: vec!["plugin:manage".into()],
    }
}

fn protected(root: &Path, bundle: Bundle, management: bool) -> Result<Bundle> {
    let manifest = if bundle.manifest.contains("[plugin]\n") {
        bundle.manifest.replace(
            "[plugin]\n",
            "[plugin]\nrequired_permissions=['plugin:manage']\n",
        )
    } else {
        bundle.manifest.replacen(
            "[plugin.",
            "[plugin]\nrequired_permissions=['plugin:manage']\n[plugin.",
            1,
        )
    };
    let manifest = format!("{manifest}\n[plugin.capabilities]\nmanagement={management}\n");
    std::fs::write(root.join("aio-plugin.toml"), manifest)?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        bundle.git,
        bundle.commit,
        bundle.version,
    )
}

fn request(path: &str) -> execution::Request {
    execution::Request {
        method: "GET".into(),
        path: path.into(),
        query: None,
        headers: vec![],
        body: vec![],
    }
}

fn process_client(root: &Path, start: &process::Start) -> Result<(reqwest::Client, String)> {
    let directory = root.join(start.id());
    let config: Configuration =
        serde_json::from_slice(&std::fs::read(directory.join("grant/config.json"))?)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .unix_socket(directory.join("broker/gateway.sock"))
        .timeout(Duration::from_secs(5))
        .build()?;
    Ok((client, config.ingress_token))
}

async fn broker(
    client: &reqwest::Client,
    token: &str,
    context: &str,
    method: &str,
    path: &str,
) -> Result<reqwest::Response> {
    Ok(client
        .post("http://localhost/management/invoke")
        .header("x-aio-token", token)
        .header("x-aio-context", context)
        .json(&json!({"method":method,"path":path}))
        .send()
        .await?)
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL、正式 WIT fixture 和专用监督器 socket"]
async fn wasm_and_sidecar_management_share_trust_context_and_revocation() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.path().ends_with("component_market_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let process_root =
        PathBuf::from(std::env::var_os("AIO_PROCESS_ROOT").context("缺少专用进程目录")?);
    let supervisor_socket = PathBuf::from(
        std::env::var_os("AIO_PLUGIN_SUPERVISOR_SOCKET").context("缺少专用监督器 socket")?,
    );
    let listener = tokio::net::UnixListener::bind(&supervisor_socket)?;
    let supervisor = Arc::new(process_publication_tests::Supervisor::default());
    let app = Router::new()
        .route("/bundles/start", post(process_publication_tests::start))
        .route("/bundles/stop", post(process_publication_tests::stop))
        .with_state(supervisor);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let tenant = Uuid::new_v4().to_string();
    let wasm_root = root.path().join("wasm");
    let wasm = protected(
        &wasm_root,
        tests::package(
            &wasm_root,
            &format!("https://github.com/example/manager-wasm-{tenant}.git"),
            None,
            "1.0.0",
        )?,
        true,
    )?;
    let sidecar_root = root.path().join("sidecar");
    let sidecar = protected(
        &sidecar_root,
        process_publication_tests::package(
            &sidecar_root,
            &format!("https://github.com/example/manager-process-{tenant}.git"),
            "1.0.0",
        )?,
        true,
    )?;
    let identity = Arc::new(Identity {
        revoked: false.into(),
    });
    let mut selected = default_plugins()?;
    selected.push(Arc::new(ComponentManagementPlugin(
        [&wasm, &sidecar]
            .iter()
            .map(|bundle| TrustedManagementComponent {
                git: bundle.git.clone(),
                digest: bundle.digest.clone(),
            })
            .collect(),
    )));
    let state = RuntimeState::initialize_selected(
        crate::configuration::HostConfig {
            database_url: database.clone(),
            public_origin: "http://localhost".into(),
            cache_root: root.path().join("cache"),
            component_storage: Some(crate::configuration::ComponentStorage {
                database_url: database,
                root: root.path().join("components"),
            }),
            default_plugins: vec![],
            development: None,
            delivery: None,
            remote_capabilities: None,
        },
        with_identity(identity.clone(), selected),
        false,
    )
    .await?;
    let components = state.components()?;
    let wasm_source = components.publish(wasm.clone(), "").await?;
    components.install(&tenant, &wasm.git, None).await?;
    let source = components.publish(sidecar.clone(), "").await?;
    components.install(&tenant, &sidecar.git, None).await?;
    let wasm_authorization = components
        .services
        .enter(&session(&tenant), &wasm_source.to_string())?;
    let authorization = components
        .services
        .enter(&session(&tenant), &source.to_string())?;
    let scope = InvocationScope {
        source_id: source.to_string(),
        revision: sidecar.digest.clone(),
        context: authorization.context.clone(),
        grants: sidecar.verify()?.manifest().plugin.capabilities.clone(),
    };
    let start = process::Start {
        source,
        tenant: tenant.clone(),
        revision: sidecar.digest.clone(),
    };
    let (client, token) = process_client(&process_root, &start)?;
    let response = components
        .handle(
            wasm_source,
            &tenant,
            &wasm.digest,
            request("/management"),
            wasm_authorization.context.clone(),
        )
        .await?;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let response = broker(
        &client,
        &token,
        &authorization.context.request_id,
        "GET",
        "/api/runtime/catalog",
    )
    .await?
    .error_for_status()?
    .json::<Value>()
    .await?;
    assert_eq!(response["status"], 200);
    let bytes: Vec<u8> = serde_json::from_value(response["body"].clone())?;
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes)?["data"]["tenant"]["id"],
        tenant
    );
    for context in ["forged", &wasm_authorization.context.request_id] {
        assert_eq!(
            broker(&client, &token, context, "GET", "/api/runtime/catalog")
                .await?
                .status(),
            403
        );
    }
    assert_eq!(
        broker(
            &client,
            "forged",
            &authorization.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await?
        .status(),
        403
    );
    let child = components.services.delegate(
        &wasm_authorization.context,
        &wasm_source.to_string(),
        &source.to_string(),
    )?;
    assert_eq!(
        broker(
            &client,
            &token,
            &child.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await?
        .status(),
        403
    );
    let background = components.services.background(
        &tenant,
        "manager",
        &wasm_source.to_string(),
        &source.to_string(),
        vec!["plugin:manage".into()],
    )?;
    assert_eq!(
        broker(
            &client,
            &token,
            &background.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await?
        .status(),
        403
    );
    identity.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        broker(
            &client,
            &token,
            &authorization.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await?
        .status(),
        403
    );
    assert!(
        components
            .services
            .manage(&scope, request("/api/runtime/catalog"))
            .await
            .is_err()
    );
    identity.revoked.store(false, Ordering::SeqCst);
    assert_eq!(
        broker(
            &client,
            &token,
            &authorization.context.request_id,
            "POST",
            &format!("/api/runtime/plugins/{source}/disable")
        )
        .await?
        .status(),
        403
    );
    let mut mutation = request(&format!(
        "/management/api/runtime/plugins/{wasm_source}/disable"
    ));
    mutation.method = "POST".into();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(3),
            components.handle(
                wasm_source,
                &tenant,
                &wasm.digest,
                mutation,
                wasm_authorization.context.clone()
            )
        )
        .await??
        .status,
        403
    );
    drop(authorization);
    assert_eq!(
        broker(
            &client,
            &token,
            &scope.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await?
        .status(),
        403
    );
    let second_root = root.path().join("second");
    let second = protected(
        &second_root,
        process_publication_tests::package(&second_root, &sidecar.git, "2.0.0")?,
        false,
    )?;
    components.publish(second.clone(), "").await?;
    components.install(&tenant, &sidecar.git, None).await?;
    let next = process::Start {
        revision: second.digest.clone(),
        ..start
    };
    let (next_client, next_token) = process_client(&process_root, &next)?;
    let next_authorization = components
        .services
        .enter(&session(&tenant), &source.to_string())?;
    assert_eq!(
        broker(
            &next_client,
            &next_token,
            &next_authorization.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await?
        .status(),
        403
    );
    assert!(
        broker(
            &client,
            &token,
            &next_authorization.context.request_id,
            "GET",
            "/api/runtime/catalog"
        )
        .await
        .is_err()
    );
    state.shutdown().await?;
    server.abort();
    let _ = server.await;
    tokio::fs::remove_file(supervisor_socket).await?;
    Ok(())
}
