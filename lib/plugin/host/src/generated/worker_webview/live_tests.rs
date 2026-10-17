//! 使用真实 PostgreSQL、HTTP 和 WebSocket 验证协议提供者在线增删。

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use futures_util::StreamExt;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

use super::{
    WebviewProtocolProvider, WorkerWebviewService, WorkerWebviewServiceImpl,
    model::{ViewOwner, WebviewRuntime},
};
use crate::{
    generated::worker::WorkerServiceImpl,
    identity::{IdentityProvider, SessionContext},
    runtime::server::plugins::{RuntimeAssembly, RuntimeHost, WorkerWebviewsPlugin},
};

struct Fixture {
    pool: sqlx::PgPool,
    views: Arc<WorkerWebviewServiceImpl>,
}

struct Identity;
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, _: &axum::http::HeaderMap) -> Result<Option<SessionContext>> {
        Ok(None)
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
}

impl Plugin<RuntimeAssembly> for Fixture {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add_value(self.pool.clone());
            assembly.services.add::<WorkerServiceImpl>();
            assembly.services.add_builder(self.views.clone());
            assembly
                .services
                .bind::<dyn WorkerWebviewService, WorkerWebviewServiceImpl>();
            assembly
                .services
                .add_value(Identity)
                .bind::<dyn IdentityProvider, Identity>();
            Ok(())
        })
    }
}

struct Sensor;
impl WebviewProtocolProvider for Sensor {
    fn protocols(&self) -> Vec<WebviewRuntime> {
        vec![runtime()]
    }
}
impl Plugin<RuntimeAssembly> for Sensor {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Sensor)
                .bind::<dyn WebviewProtocolProvider, Sensor>();
            Ok(())
        })
    }
}

fn runtime() -> WebviewRuntime {
    WebviewRuntime {
        protocol: "sensor".into(),
        capability: "sensor.stream".into(),
        headless_capability: None,
        document: None,
    }
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn protocol_unload_closes_live_socket_and_reinstall_recovers() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = sqlx::PgPool::connect(&database).await?;
    let schema = format!("protocol_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let mut url = reqwest::Url::parse(&database)?;
    url.query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let pool = sqlx::PgPool::connect(url.as_str()).await?;
    sqlx::raw_sql(include_str!("../worker/schema.sql"))
        .execute(&pool)
        .await?;
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&pool)
        .await?;
    let views = dill::Catalog::builder()
        .add_value(pool.clone())
        .add::<WorkerWebviewServiceImpl>()
        .build()
        .get_one::<WorkerWebviewServiceImpl>()?;
    let device = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO worker_devices(id,token_hash,label,platform,tenant_id,user_id,state,capabilities) VALUES($1,$2,'fixture','linux','tenant','user','active','[]')")
        .bind(&device).bind(super::util::digest(&token)).execute(&pool).await?;
    let root = tempfile::tempdir()?;
    let base: Vec<DynPlugin<RuntimeAssembly>> = vec![
        Arc::new(Fixture {
            pool: pool.clone(),
            views: views.clone(),
        }),
        Arc::new(WorkerWebviewsPlugin),
    ];
    let mut installed = base.clone();
    installed.push(Arc::new(Sensor));
    let host = RuntimeHost::initialize(
        crate::configuration::HostConfig {
            database_url: url.to_string(),
            public_origin: "http://127.0.0.1".into(),
            cache_root: root.path().into(),
            component_storage: None,
            default_plugins: vec![],
            delivery: None,
            development: None,
            remote_capabilities: None,
        },
        installed.clone(),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = host.router();
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let access = || {
            client
                .post(format!(
                    "http://{address}/api/runtime/workers/webviews/access"
                ))
                .bearer_auth(&token)
                .json(&json!({"enabled":true,"runtime":"sensor"}))
                .send()
        };
        access().await?.error_for_status()?;
        let request = || -> Result<_> {
            let mut request = format!("ws://{address}/api/runtime/workers/webviews/channel")
                .into_client_request()?;
            request
                .headers_mut()
                .insert("authorization", format!("Bearer {token}").parse()?);
            Ok(request)
        };
        let (mut socket, _) = connect_async(request()?).await?;
        let owner = ViewOwner {
            runtime: runtime(),
            tenant: "tenant".into(),
            user: "user".into(),
            session: "session".into(),
            source: "sensor-plugin".into(),
            revision: "revision".into(),
            mount: "mount".into(),
        };
        let view = views
            .create(&owner, &device, "/")
            .await
            .map_err(|_| anyhow::anyhow!("自定义协议创建失败"))?;
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(frame) = socket.next().await {
                let frame = frame?;
                if frame.is_text() {
                    let message: serde_json::Value = serde_json::from_str(frame.to_text()?)?;
                    assert_eq!(message["runtime"], "sensor");
                    assert_eq!(message["sessionId"], view);
                    return Ok::<_, anyhow::Error>(());
                }
            }
            anyhow::bail!("连接没有收到打开消息")
        })
        .await??;
        // 仅卸载 Provider；路由插件的聚合注入依赖必须自动重建并排空旧连接。
        tokio::time::timeout(Duration::from_secs(5), host.reconfigure(base)).await??;
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(frame) = socket.next().await {
                match frame {
                    Ok(frame) if !frame.is_close() => {}
                    _ => break,
                }
            }
        })
        .await?;
        assert!(views.authorize(&owner, &view).await.is_err());
        assert_eq!(access().await?.status(), 403);
        assert!(connect_async(request()?).await.is_err());
        host.reconfigure(installed).await?;
        access().await?.error_for_status()?;
        let (mut recovered, _) = connect_async(request()?).await?;
        // 重新装配可创建新会话，但不会复活旧视图。
        let next = views
            .create(&owner, &device, "/")
            .await
            .map_err(|_| anyhow::anyhow!("重新装配未恢复通道"))?;
        assert_ne!(view, next);
        assert!(views.authorize(&owner, &view).await.is_err());
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = recovered.next().await.context("恢复后的连接提前关闭")??;
                if frame.is_text() {
                    let message: serde_json::Value = serde_json::from_str(frame.to_text()?)?;
                    assert_eq!(message["runtime"], "sensor");
                    assert_eq!(message["sessionId"], next);
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await??;
        recovered.close(None).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let stopped = host.shutdown().await;
    server.abort();
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result?;
    stopped
}
