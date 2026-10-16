use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_enhancement::{Enhancement, Language, Package};
use serde_json::{Value, json};

use crate::runtime::server::plugins::*;

pub(super) const BOOTSTRAP: &str = "/v1/evolution/bootstrap";
pub(super) const OBSERVE: &str = "/v1/evolution/observe";
pub(super) const CONTROL: &str = "/v1/evolution/control";
pub(super) const MODEL: &str = "/v1/model/enhancement/source";
pub(super) const MATH: &str = "/v1/math/double";

pub(super) fn capability(operation: &str) -> String {
    format!("/v1/runtime/enhancements/{operation}")
}

// 这里只替换候选源码生成器；实际双语言执行、数据库、权限和协议都使用正式实现。
#[derive(Default)]
pub(super) struct Generator {
    pub calls: AtomicUsize,
    pub active: AtomicUsize,
    pub entered: tokio::sync::Notify,
}

struct Running<'a>(&'a AtomicUsize);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for Generator {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        vec![RuntimeCapability {
            path: MODEL.into(),
            description: "测试候选生成器".into(),
            effect: CapabilityEffect::Write,
            input_schema: schemars::schema_for!(Value),
            output_schema: schemars::schema_for!(Value),
        }]
    }

    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_add(1, Ordering::SeqCst);
        let _running = Running(&self.active);
        self.entered.notify_one();
        let requirement = request.input["requirement"].as_str().unwrap_or_default();
        if requirement == "wait" {
            std::future::pending::<()>().await;
        }
        let source = if request.input["target"]["path"] == BOOTSTRAP {
            format!("{}\n// 新策略版本\n", policy(Language::TypeScript, false))
        } else {
            match (request.input["target"]["language"] == "rhai", requirement) {
                (_, "invalid") => "invalid { script",
                (true, "break") => {
                    "fn invoke(input, context) { if input == 7 { throw \"fault\"; } input * 2 }"
                }
                (false, "break") => {
                    "export function invoke(input: number) { if (input === 7) throw new Error('fault'); return input * 2; }"
                }
                (true, _) => "fn invoke(input, context) { input * 2 }",
                (false, _) => "export function invoke(input: number) { return input * 2; }",
            }.to_owned()
        };
        Ok(json!({"source":source}))
    }
}

pub(super) struct GeneratorPlugin(pub Arc<Generator>);
impl Plugin<RuntimeAssembly> for GeneratorPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn CapabilityProvider, Generator>();
            Ok(())
        })
    }
}

pub(super) fn policy(language: Language, observer: bool) -> &'static str {
    match (language, observer) {
        (Language::Rhai, false) => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/plugin/evolution/bootstrap.rhai"
        )),
        (Language::TypeScript, false) => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/plugin/evolution/bootstrap.ts"
        )),
        (Language::Rhai, true) => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/plugin/evolution/observe.rhai"
        )),
        (Language::TypeScript, true) => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../docs/plugin/evolution/observe.ts"
        )),
    }
}

pub(super) fn target(language: Language) -> Value {
    json!({"path":MATH,"language":language,"source":"","calls":[],"capabilities":[],"examples":[{"input":3,"output":6}]})
}

pub(super) fn package(language: Language) -> Result<Package> {
    let contracts: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/plugin/evolution/contracts.json"
    )))?;
    let mut package = super::super::support::package(Language::Rhai);
    for (name, observer) in [("bootstrap", false), ("observe", true)] {
        let mut entry = contracts[name].clone();
        entry["language"] = serde_json::to_value(language)?;
        entry["source"] = json!(policy(language, observer));
        package.enhancements.push(serde_json::from_value(entry)?);
    }
    let head = json!({"revision":"before","generation":1});
    let control: Enhancement = serde_json::from_value(json!({
        "path":CONTROL,"language":"typescript","source":"export function invoke(input) { return call_capability(input.path, input.value); }",
        "capabilities":[capability("head"),capability("package"),capability("propose"),capability("activate"),capability("rollback"),capability("history"),capability("observe")],
        "examples":[{"input":{"path":capability("head"),"value":{}},"output":head,"capabilities":[json!({"path":capability("head"),"input":{},"output":head})]}]
    }))?;
    package.enhancements.push(control);
    Ok(package)
}
