use anyhow::{Context, Result, ensure};
use az_plugin_development::DevHostSession;
use az_plugin_host::{
    configuration::{ComponentStorage, HostConfig},
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, bootstrap_document},
};
use std::{path::PathBuf, sync::Arc};

struct DevelopmentIdentity;

#[async_trait::async_trait]
impl IdentityProvider for DevelopmentIdentity {
    async fn can_publish(
        &self,
        _: &az_plugin_host::identity::SessionContext,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == "development" && user == "developer")
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "development" && self.member_active(tenant, user).await?)
    }
    async fn install_permissions(&self, _: &str, _: &[String]) -> Result<()> {
        Ok(())
    }
    async fn authenticate(&self, _: &axum::http::HeaderMap) -> Result<Option<SessionContext>> {
        Ok(Some(SessionContext {
            session_id: "development".into(),
            user_id: "developer".into(),
            account: "developer".into(),
            display_name: "Developer".into(),
            tenant_id: "development".into(),
            tenant_label: "Sandbox".into(),
            permissions: vec!["*".into()],
        }))
    }
}

pub async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.as_slice() == ["--version"] {
        println!("aio-host {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    ensure!(
        args.len() == 2 && args[0] == "--session",
        "用法: aio-host --session <开发会话文件>"
    );
    let session: DevHostSession = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let web = std::env::var_os("AIO_DEV_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or(
            std::env::current_exe()?
                .parent()
                .context("宿主路径无效")?
                .join("web"),
        );
    ensure!(
        web.join("index.html").is_file(),
        "开发宿主缺少与 CLI 配套的预编译 Web 资源: {}",
        web.display()
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, session.port))
        .await
        .context("开发宿主端口被占用")?;
    let origin = format!("http://{}", listener.local_addr()?);
    az_plugin_host::runtime::server::development::claim_database(&session).await?;
    let state = RuntimeState::initialize(
        HostConfig {
            database_url: session.database_url.clone(),
            cache_root: session.root.join("artifacts"),
            public_origin: origin.clone(),
            component_storage: Some(ComponentStorage {
                database_url: session.database_url.clone(),
                root: session.root.join("components"),
            }),
            default_plugins: vec![],
            delivery: None,
            development: Some(session.clone()),
        },
        Arc::new(DevelopmentIdentity),
    )
    .await?;
    let application = az_plugin_host::static_files::application(web).layer(
        axum::middleware::from_fn_with_state(state.clone(), bootstrap_document),
    );
    let expected_origin = origin.clone();
    let expected_authority = listener.local_addr()?.to_string();
    let router = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .merge(az_plugin_host::runtime::server::development::router(
            state.clone(),
        )?)
        .merge(az_plugin_host::runtime::server::router(state))
        .fallback_service(application)
        .layer(axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: axum::middleware::Next| {
                let origin = expected_origin.clone();
                let authority = expected_authority.clone();
                async move {
                    use axum::{
                        http::{Method, StatusCode, header},
                        response::IntoResponse,
                    };
                    let host = request
                        .headers()
                        .get(header::HOST)
                        .and_then(|value| value.to_str().ok());
                    let source = request
                        .headers()
                        .get(header::ORIGIN)
                        .and_then(|value| value.to_str().ok());
                    let read = matches!(*request.method(), Method::GET | Method::HEAD);
                    if host != Some(authority.as_str())
                        || source.is_some_and(|value| value != origin && !(value == "null" && read))
                    {
                        return (StatusCode::FORBIDDEN, "开发宿主仅接受本地同源请求")
                            .into_response();
                    }
                    // 开发身份由回环宿主提供，通信桥仍使用正式的会话挂载边界。
                    request.headers_mut().insert(
                        axum::http::header::COOKIE,
                        axum::http::HeaderValue::from_static("aio_development=local"),
                    );
                    next.run(request).await
                }
            },
        ));
    std::fs::write(
        session.root.join("host.json"),
        serde_json::to_vec(&serde_json::json!({
            "url": origin, "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION")
        }))?,
    )?;
    println!("AIO Sandbox {origin}");
    use std::future::IntoFuture;
    // SSE 长连接不等待浏览器关闭；退出时回收路由、任务和开发服务授权。
    tokio::select! {
        result = axum::serve(listener, router).into_future() => result?,
        _ = shutdown() => {}
    }
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
