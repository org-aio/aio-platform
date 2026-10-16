use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use az_plugin_contract::{
    CapabilityGrants, EnhancementRequest, EnhancementResponse, InvocationScope, RequestContext,
};
use az_plugin_enhancement::{
    Enhancement, Example, Language, Package, PreparedPackage, ScriptContext,
};
use az_plugin_runtime::{
    ComponentEngine, HostServices, InvocationResources,
    bindings::aio::plugin::transport::{Request, Response},
};
use serde_json::json;

struct Host(RwLock<PreparedPackage>);

#[async_trait]
impl HostServices for Host {
    async fn authorize(&self, _: &InvocationScope, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn manage(&self, _: &InvocationScope, _: Request) -> Result<Response> {
        anyhow::bail!("测试未授予管理能力")
    }
    async fn enhance(
        &self,
        scope: &InvocationScope,
        request: EnhancementRequest,
    ) -> Result<EnhancementResponse> {
        let package = self.0.read().map_err(|_| anyhow::anyhow!("增强锁损坏"))?;
        let context = ScriptContext {
            tenant_id: scope.context.tenant_id.clone().context("缺少租户")?,
            source_id: scope.source_id.clone(),
        };
        let output = package.invoke(&request.path, &request.input, &context)?;
        Ok(EnhancementResponse {
            revision: package.revision().into(),
            output,
        })
    }
}

fn package(language: Language, source: &str) -> Result<PreparedPackage> {
    az_plugin_enhancement::default_runtime()?.prepare(Package {
        schema_version: 1,
        enhancements: vec![Enhancement {
            path: "/v1/math/double".into(),
            language,
            source: source.into(),
            examples: vec![Example {
                input: json!(3),
                output: json!(6),
            }],
        }],
    })
}

fn request() -> Request {
    Request {
        method: "POST".into(),
        path: "/enhancement".into(),
        query: None,
        headers: vec![],
        body: b"7".to_vec(),
    }
}

#[tokio::test]
#[ignore = "需要已构建的 enhancements WIT fixture"]
async fn same_wasm_instance_receives_runtime_enhancements_without_recompilation() -> Result<()> {
    let engine = ComponentEngine::new()?;
    let bytes = std::fs::read(std::env::var("AIO_TEST_ENHANCEMENT_COMPONENT")?)?;
    let compiled = engine.compile(&bytes).await?;
    let host = Arc::new(Host(RwLock::new(package(
        Language::Rhai,
        "fn invoke(input, context) { input * 2 }",
    )?)));
    let context = RequestContext {
        tenant_id: Some("tenant".into()),
        ..Default::default()
    };
    let scope = InvocationScope {
        source_id: "source".into(),
        revision: "unchanged-plugin".into(),
        context: context.clone(),
        grants: CapabilityGrants {
            enhancements: vec!["/v1/math/double".into()],
            ..Default::default()
        },
    };
    let resources = InvocationResources {
        services: Some(host.clone()),
        ..Default::default()
    };
    let mut guest = engine
        .instantiate(&compiled, scope.clone(), resources.clone())
        .await?;
    let first = guest.handle(request(), context.clone()).await?;
    assert_eq!(first.status, 200);
    let first: EnhancementResponse = serde_json::from_slice(&first.body)?;
    assert_eq!(first.output, json!(14));
    *host.0.write().map_err(|_| anyhow::anyhow!("增强锁损坏"))? = package(
        Language::TypeScript,
        "export function invoke(input: number) { return input * 2; }",
    )?;
    let second = guest.handle(request(), context.clone()).await?;
    assert_eq!(second.status, 200);
    let second: EnhancementResponse = serde_json::from_slice(&second.body)?;
    assert_eq!(second.output, json!(14));
    assert_ne!(first.revision, second.revision);
    let mut denied = scope;
    denied.grants.enhancements.clear();
    let mut denied = engine.instantiate(&compiled, denied, resources).await?;
    assert_eq!(denied.handle(request(), context).await?.status, 403);
    Ok(())
}
