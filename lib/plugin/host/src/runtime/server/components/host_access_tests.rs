//! 使用真实 Wasm 与 Unix socket，验证交互授权不会被进程或开发装配绕过。
use super::{Components, execution::Request};
use crate::{
    authorization::AuthorizationProvider,
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, request_context},
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    http::HeaderMap,
    routing::{get, post},
};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{
    RequestContext,
    process::{Configuration, ServiceRequest},
};
use az_plugin_development::DevArtifact;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

#[derive(Default)]
struct Identity {
    revoked: AtomicBool,
}
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, _: &HeaderMap) -> Result<Option<SessionContext>> {
        Ok(None)
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == "development" && matches!(user, "admin" | "member"))
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == format!("session:{user}") && self.member_active(tenant, user).await?)
    }
}
#[async_trait::async_trait]
impl AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn session_has_permissions(
        &self,
        session: &str,
        tenant: &str,
        user: &str,
        required: &[String],
    ) -> Result<bool> {
        Ok(user == "admin"
            && !self.revoked.load(Ordering::SeqCst)
            && self.session_active(session, tenant, user).await?
            && required == ["dictionary:manage"])
    }
}
fn session(user: &str) -> SessionContext {
    SessionContext {
        session_id: format!("session:{user}"),
        tenant_id: "development".into(),
        user_id: user.into(),
        account: user.into(),
        display_name: user.into(),
        tenant_label: "Development".into(),
        permissions: vec![],
    }
}
fn protect(root: &Path, bundle: Bundle) -> Result<Bundle> {
    let field = if bundle.verify()?.manifest().plugin.runtime.process.is_some() {
        "required_permissions=['dictionary:manage']\nsettings_page='settings'\n"
    } else {
        "required_permissions=['dictionary:manage']\n"
    };
    let manifest = if bundle.manifest.contains("[plugin]\n") {
        bundle
            .manifest
            .replace("[plugin]\n", &format!("[plugin]\n{field}"))
    } else {
        bundle
            .manifest
            .replacen("[plugin.", &format!("[plugin]\n{field}[plugin."), 1)
    };
    std::fs::write(root.join("aio-plugin.toml"), manifest)?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        bundle.git,
        bundle.commit,
        bundle.version,
    )
}
fn artifact(root: &Path, bundle: &Bundle, generation: u64) -> Result<DevArtifact> {
    let verified = bundle.verify()?;
    Ok(DevArtifact {
        workspace: root.into(),
        source: bundle.git.clone(),
        content_digest: bundle.digest.clone(),
        frontend: root.join(&verified.manifest().plugin.frontend.path),
        backend: root.join(&verified.manifest().plugin.runtime.artifact),
        backend_digest: bundle.digest.clone(),
        endpoint: None,
        generation,
    })
}
async fn call(
    components: &Components,
    source: Uuid,
    digest: &str,
    context: RequestContext,
) -> Result<Vec<u8>> {
    Ok(components
        .handle(
            source,
            "development",
            digest,
            Request {
                method: "POST".into(),
                path: "/echo".into(),
                query: None,
                headers: vec![
                    az_plugin_contract::transport::Header {
                        name: "content-type".into(),
                        value: "application/json".into(),
                    },
                    az_plugin_contract::transport::Header {
                        name: "x-aio-user-id".into(),
                        value: "forged".into(),
                    },
                ],
                body: b"{\"ok\":true}".to_vec(),
            },
            context,
        )
        .await?
        .body)
}

struct LocalProcess {
    client: reqwest::Client,
    token: String,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Drop for LocalProcess {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn start_local(
    components: &Components,
    root: &Path,
    bundle: &Bundle,
    generation: u64,
) -> Result<LocalProcess> {
    let artifact = artifact(root, bundle, generation)?;
    let launch = components
        .prepare_development(&artifact, Arc::new(bundle.verify()?))
        .await?;
    let configuration: Configuration = serde_json::from_slice(&std::fs::read(
        launch
            .environment
            .get("AIO_PLUGIN_CONFIG")
            .context("缺少配置")?,
    )?)?;
    let listener = tokio::net::UnixListener::bind(launch.socket.context("缺少 socket")?)?;
    let app = Router::new()
        .route("/aio/describe", get(|| async { Json(json!({"label":"Protected","pages":[
            {"id":"fixture","label":"Fixture","entry":"index.html","scene":null,"menu_path":[],"permission":null,"surface":"fullscreen"},
            {"id":"settings","label":"Settings","entry":"index.html","scene":null,"menu_path":[],"permission":null,"surface":"fullscreen"}
        ]})) }))
        .route("/echo", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
            assert_eq!(headers.get("x-aio-user-id").and_then(|value| value.to_str().ok()), Some("admin"));
            Json(body)
        }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let local = LocalProcess {
        client: reqwest::Client::builder()
            .no_proxy()
            .unix_socket(configuration.broker_socket)
            .build()?,
        token: configuration.ingress_token,
        server,
    };
    components
        .activate_development(&artifact, Arc::new(bundle.verify()?), &bundle.git)
        .await?;
    Ok(local)
}
async fn invoke(local: &LocalProcess, request: &ServiceRequest) -> Result<reqwest::Response> {
    Ok(local
        .client
        .post("http://localhost/invoke")
        .header("x-aio-token", &local.token)
        .json(request)
        .send()
        .await?)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和已构建 WIT Component"]
async fn protected_wasm_and_sidecar_require_live_interaction_through_the_broker() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.path().ends_with("component_market_test"),
        "只接受本机独立组件测试库"
    );
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity::default());
    let state = RuntimeState::isolated_component_test(
        identity.clone(),
        &database,
        "http://localhost",
        root.path(),
    )
    .await?;
    let components = state.components()?;
    let target_git = format!(
        "https://github.com/example/protected-{}.git",
        Uuid::new_v4()
    );
    let target = Uuid::new_v5(&Uuid::NAMESPACE_URL, target_git.as_bytes());
    let wasm_root = root.path().join("wasm");
    let wasm = protect(
        &wasm_root,
        super::tests::package(&wasm_root, &target_git, None, "1.0.0")?,
    )?;
    components
        .activate_development(
            &artifact(&wasm_root, &wasm, 1)?,
            Arc::new(wasm.verify()?),
            &target_git,
        )
        .await?;

    let caller_git = format!("https://github.com/example/caller-{}.git", Uuid::new_v4());
    let caller = Uuid::new_v5(&Uuid::NAMESPACE_URL, caller_git.as_bytes());
    let caller_root = root.path().join("caller");
    let mut caller_bundle =
        super::process_publication_tests::package(&caller_root, &caller_git, "1.0.0")?;
    caller_bundle.manifest = caller_bundle.manifest.replace(
        "[plugin.runtime.process]",
        &format!("[plugin.runtime.process]\nservices=[{target_git:?}]"),
    );
    std::fs::write(caller_root.join("aio-plugin.toml"), &caller_bundle.manifest)?;
    let caller_bundle = Bundle::from_directory(
        &caller_root,
        "aio-plugin.toml",
        caller_bundle.git,
        caller_bundle.commit,
        caller_bundle.version,
    )?;
    let local = start_local(&components, &caller_root, &caller_bundle, 1).await?;
    for user in ["admin", "member"] {
        sqlx::query("INSERT INTO component_process_actors(source_id,tenant_id,user_id) VALUES($1,'development',$2)").bind(caller).bind(user).execute(&components.pool).await?;
    }
    let process_root = root.path().join("target");
    let process_bundle = protect(
        &process_root,
        super::process_publication_tests::package(&process_root, &target_git, "2.0.0")?,
    )?;
    let mut target_process = None;
    for bundle in [&wasm, &process_bundle] {
        if bundle.digest == process_bundle.digest {
            target_process = Some(start_local(&components, &process_root, bundle, 2).await?);
        }
        let admin = session("admin");
        let parent = components.services.enter(&admin, &caller.to_string())?;
        let delegated = components.services.delegate(
            &parent.context,
            &caller.to_string(),
            &target.to_string(),
        )?;
        let request = ServiceRequest {
            target: target_git.clone(),
            method: "POST".into(),
            path: "/echo".into(),
            body: json!({"ok":true}),
            tenant_id: "development".into(),
            user_id: "admin".into(),
            context_id: Some(parent.context.request_id.clone()),
            interactive: true,
        };
        let response: Value = invoke(&local, &request)
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(response["body"]["ok"], true);
        assert_eq!(
            call(
                &components,
                target,
                &bundle.digest,
                delegated.context.clone()
            )
            .await?,
            b"{\"ok\":true}"
        );
        assert!(
            call(&components, target, &bundle.digest, parent.context.clone())
                .await
                .is_err()
        );
        let mut forged = delegated.context.clone();
        forged.request_id = Uuid::new_v4().to_string();
        assert!(
            call(&components, target, &bundle.digest, forged)
                .await
                .is_err()
        );
        let mut forged = delegated.context.clone();
        forged.user_id = Some("member".into());
        assert!(
            call(&components, target, &bundle.digest, forged)
                .await
                .is_err()
        );
        let background = components.services.background(
            "development",
            "admin",
            &caller.to_string(),
            &target.to_string(),
            vec!["dictionary:manage".into()],
        )?;
        assert!(
            call(
                &components,
                target,
                &bundle.digest,
                background.context.clone()
            )
            .await
            .is_err()
        );
        let mut background_request = request;
        background_request.interactive = false;
        assert_eq!(invoke(&local, &background_request).await?.status(), 403);
        background_request.interactive = true;

        let member = components
            .services
            .enter(&session("member"), &caller.to_string())?;
        let mut member_request = ServiceRequest {
            user_id: "member".into(),
            context_id: Some(member.context.request_id.clone()),
            ..background_request
        };
        assert_eq!(invoke(&local, &member_request).await?.status(), 403);
        member_request.user_id = "admin".into();
        assert_eq!(invoke(&local, &member_request).await?.status(), 403);
        member_request.context_id = Some(parent.context.request_id.clone());
        identity.revoked.store(true, Ordering::SeqCst);
        assert_eq!(invoke(&local, &member_request).await?.status(), 403);
        assert!(
            call(
                &components,
                target,
                &bundle.digest,
                delegated.context.clone()
            )
            .await
            .is_err()
        );
        identity.revoked.store(false, Ordering::SeqCst);
        member_request.tenant_id = "other".into();
        assert_eq!(invoke(&local, &member_request).await?.status(), 403);
        drop(parent);
        assert!(
            call(
                &components,
                target,
                &bundle.digest,
                delegated.context.clone()
            )
            .await
            .is_err()
        );

        let authorized = request_context::catalog_value(&state, &admin)
            .await
            .map_err(|_| anyhow::anyhow!("测试管理员目录失败"))?;
        assert!(
            authorized
                .pages
                .iter()
                .any(|page| page.id == format!("component:{target}:fixture"))
        );
        if bundle.digest == process_bundle.digest {
            assert!(
                authorized
                    .account_items
                    .iter()
                    .any(|item| item.page_id == format!("component:{target}:fixture"))
            );
            assert!(
                authorized
                    .plugin_settings
                    .iter()
                    .any(|item| item.source_id == target.to_string())
            );
        }
        // 账户入口、设置入口和页面版本都必须与页面一起隐藏；安装条目保留。
        let catalog = request_context::catalog_value(&state, &session("member"))
            .await
            .map_err(|_| anyhow::anyhow!("测试目录失败"))?;
        let prefix = format!("component:{target}:");
        assert!(
            catalog
                .pages
                .iter()
                .all(|page| !page.id.starts_with(&prefix))
        );
        assert!(
            catalog
                .account_items
                .iter()
                .all(|item| !item.page_id.starts_with(&prefix))
        );
        assert!(
            catalog
                .plugin_settings
                .iter()
                .all(|item| item.source_id != target.to_string())
        );
        assert!(
            catalog
                .page_versions
                .keys()
                .all(|id| !id.starts_with(&prefix))
        );
        assert!(
            catalog
                .plugins
                .iter()
                .any(|plugin| plugin.source_id == target.to_string())
        );
        assert!(
            !components
                .permissions("development")
                .await?
                .contains(&"dictionary:manage".into())
        );
        let mut cookie = HeaderMap::new();
        cookie.insert("cookie", "session=admin".parse()?);
        assert!(
            super::frontend::mount(
                &state,
                &cookie,
                &session("member"),
                &format!("{prefix}fixture")
            )
            .await
            .is_err()
        );
        super::frontend::mount(&state, &cookie, &admin, &format!("{prefix}fixture")).await?;
        if bundle.digest != wasm.digest {
            let authorization = components.services.enter(&admin, &target.to_string())?;
            assert!(
                call(
                    &components,
                    target,
                    &wasm.digest,
                    authorization.context.clone()
                )
                .await
                .is_err()
            );
        }
    }
    state.shutdown().await?;
    drop(target_process);
    drop(local);
    Ok(())
}
