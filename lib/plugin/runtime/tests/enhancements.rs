#![cfg(feature = "wasm")]

use az_plugin_contract::transport::{Request, Response};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use az_plugin_contract::{
    CapabilityGrants, EnhancementRequest, EnhancementResponse, InvocationScope, RequestContext,
};
use az_plugin_enhancement::{
    Enhancement, Example, Language, Package, PreparedPackage, ScriptContext,
};
use az_plugin_runtime::{ComponentEngine, HostServices, InvocationResources};
use serde_json::json;

struct Host(RwLock<PreparedPackage>);

struct Math;
impl az_plugin_enhancement::CapabilityCaller for Math {
    fn available(&self, path: &str) -> bool {
        path == "/v1/runtime/double"
    }
    fn call(
        &self,
        _: &str,
        input: &serde_json::Value,
        control: &az_plugin_enhancement::InvocationControl,
    ) -> Result<serde_json::Value> {
        control.check()?;
        Ok(json!(input.as_i64().context("测试能力需要整数")? * 2))
    }
}

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
        let output = package
            .invocation(
                context,
                scope.grants.enhancements.clone(),
                az_plugin_enhancement::InvocationCapabilities {
                    grants: scope.grants.runtime.clone(),
                    caller: Some(Arc::new(Math)),
                },
                None,
            )
            .invoke(&request.path, &request.input)?;
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
            capabilities: vec![],
            calls: vec![],
            path: "/v1/math/double".into(),
            language,
            source: source.into(),
            examples: vec![Example {
                capabilities: vec![],
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
            enhancements: vec!["/v1/math/double".into(), "/v1/math/base".into()],
            runtime: vec!["/v1/runtime/double".into()],
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
    let composed = az_plugin_enhancement::default_runtime()?.prepare(serde_json::from_str(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../enhancement/examples/composition.json"
        )),
    )?)?;
    *host.0.write().map_err(|_| anyhow::anyhow!("增强锁损坏"))? = composed;
    let composed = guest.handle(request(), context.clone()).await?;
    assert_eq!(composed.status, 200);
    let composed: EnhancementResponse = serde_json::from_slice(&composed.body)?;
    assert_eq!(composed.output, json!(14));
    assert_ne!(composed.revision, second.revision);
    let mut missing_dependency = scope.clone();
    missing_dependency
        .grants
        .enhancements
        .retain(|path| path != "/v1/math/base");
    let mut missing_dependency = engine
        .instantiate(&compiled, missing_dependency, resources.clone())
        .await?;
    assert_eq!(
        missing_dependency
            .handle(request(), context.clone())
            .await?
            .status,
        403
    );
    let capability_package = az_plugin_enhancement::default_runtime()?.prepare(serde_json::from_value(json!({"schema_version":1,"enhancements":[{
        "path":"/v1/math/double","language":"typescript","source":"export function invoke(input) { return call_capability('/v1/runtime/double', input); }", "capabilities":["/v1/runtime/double"],
        "examples":[{"input":3,"output":6,"capabilities":[{"path":"/v1/runtime/double","input":3,"output":6}]}]
    }]}))?)?;
    *host.0.write().map_err(|_| anyhow::anyhow!("增强锁损坏"))? = capability_package;
    let through_host = guest.handle(request(), context.clone()).await?;
    assert_eq!(through_host.status, 200);
    assert_eq!(
        serde_json::from_slice::<EnhancementResponse>(&through_host.body)?.output,
        json!(14)
    );
    let mut denied_runtime = scope.clone();
    denied_runtime.grants.runtime.clear();
    let mut denied_runtime = engine
        .instantiate(&compiled, denied_runtime, resources.clone())
        .await?;
    assert_eq!(
        denied_runtime
            .handle(request(), context.clone())
            .await?
            .status,
        403
    );
    let mut denied = scope;
    denied.grants.enhancements.clear();
    let mut denied = engine.instantiate(&compiled, denied, resources).await?;
    assert_eq!(denied.handle(request(), context).await?.status, 403);
    Ok(())
}
