//! 可选依赖仍参与初始化排序、唯一性检查和在线重建。

use std::{any::Any, sync::Arc};

use anyhow::{Context, Result};
use axum::{Json, Router, body::Body, http::Request, routing::get};
use az_plugin_core::{Plugin, PluginFuture};
use tower::ServiceExt;

use super::{Resource, RuntimeAssembly, RuntimeHost, RuntimeState, tests::config};

trait Number: Any + Send + Sync {
    fn value(&self) -> u8;
}

struct Value<const N: u8>;
impl<const N: u8> Number for Value<N> {
    fn value(&self) -> u8 {
        N
    }
}

struct Source<const N: u8>;
impl<const N: u8> Plugin<RuntimeAssembly> for Source<N> {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<Value<N>>()?;
            assembly.services.bind::<dyn Number, Value<N>>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    state
                        .service::<Resource<Value<N>>>()?
                        .provide(Arc::new(Value))
                })
            });
            Ok(())
        })
    }
}

struct Observed(Option<u8>);
struct Consumer;
impl Plugin<RuntimeAssembly> for Consumer {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.optional_service::<dyn Number>();
            assembly.declare_resource::<Observed>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let value = state.optional_service::<dyn Number>()?.map(|n| n.value());
                    state
                        .service::<Resource<Observed>>()?
                        .provide(Arc::new(Observed(value)))
                })
            });
            assembly.add_routes(|state| {
                let value = state.service::<Observed>()?.0;
                Ok(Router::new().route("/observed", get(move || async move { Json(value) })))
            });
            Ok(())
        })
    }
}

async fn observed(host: &RuntimeHost) -> Result<Option<u8>> {
    let response = host
        .router()
        .oneshot(Request::get("/observed").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 128).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
async fn optional_provider_initializes_first_and_rebuilds_consumers_when_added_or_removed()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let consumer = Arc::new(Consumer);
    let host = RuntimeHost::initialize(config(root.path()), vec![consumer.clone()]).await?;
    assert_eq!(observed(&host).await?, None);
    // 消费者故意排在提供者前；只允许依赖图决定初始化顺序。
    host.reconfigure(vec![consumer.clone(), Arc::new(Source::<1>)])
        .await?;
    assert_eq!(observed(&host).await?, Some(1));
    host.reconfigure(vec![consumer.clone(), Arc::new(Source::<2>)])
        .await?;
    assert_eq!(observed(&host).await?, Some(2));
    assert!(
        host.reconfigure(vec![
            consumer.clone(),
            Arc::new(Source::<1>),
            Arc::new(Source::<2>)
        ])
        .await
        .is_err()
    );
    assert_eq!(observed(&host).await?, Some(2));
    host.reconfigure(vec![consumer]).await?;
    assert_eq!(observed(&host).await?, None);
    assert!(!root.path().join("cache").exists());
    host.shutdown().await?;
    Ok(())
}

struct Uninitialized;
impl Plugin<RuntimeAssembly> for Uninitialized {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<Value<1>>()?;
            assembly.services.bind::<dyn Number, Value<1>>();
            Ok(())
        })
    }
}

#[tokio::test]
async fn optional_provider_resolution_errors_are_not_treated_as_absence() -> Result<()> {
    let root = tempfile::tempdir()?;
    let result = RuntimeState::initialize(
        config(root.path()),
        vec![Arc::new(Consumer), Arc::new(Uninitialized)],
    )
    .await;
    let error = result.err().context("可选依赖解析失败却被当作缺省")?;
    assert!(format!("{error:#}").contains("宿主未安装所需服务插件"));
    Ok(())
}
