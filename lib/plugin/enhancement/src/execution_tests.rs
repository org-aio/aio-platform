use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    Enhancement, Example, Language, Package, ScriptContext, ScriptExecutor, ScriptInvocation,
    ScriptProgram, ScriptRuntime,
};

// 测试源码只包含倍率，故意不满足内置函数约定，证明执行来自注入的实现。
struct ScalingExecutor(Arc<AtomicUsize>);
struct ScalingProgram(i64);

impl ScriptExecutor for ScalingExecutor {
    fn supports(&self, language: Language) -> bool {
        language == Language::Rhai
    }
    fn compile(&self, source: &str) -> Result<Arc<dyn ScriptProgram>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(ScalingProgram(source.parse()?)))
    }
}

impl ScriptProgram for ScalingProgram {
    fn invoke(&self, input: &Value, _: &ScriptInvocation) -> Result<Value> {
        Ok(json!(
            input.as_i64().context("测试输入必须为整数")? * self.0
        ))
    }
}

struct ConflictingExecutor;
impl ScriptExecutor for ConflictingExecutor {
    fn supports(&self, language: Language) -> bool {
        language == Language::Rhai
    }
    fn compile(&self, _: &str) -> Result<Arc<dyn ScriptProgram>> {
        anyhow::bail!("存在歧义时不应进入编译")
    }
}

fn candidate() -> Package {
    Package {
        schema_version: 1,
        enhancements: vec![Enhancement {
            capabilities: vec![],
            calls: vec![],
            path: "/v1/math/scale".into(),
            language: Language::Rhai,
            source: "3".into(),
            examples: vec![Example {
                capabilities: vec![],
                input: json!(2),
                output: json!(6),
            }],
        }],
    }
}

fn catalog(calls: Arc<AtomicUsize>) -> dill::CatalogBuilder {
    let mut catalog = dill::CatalogBuilder::new();
    catalog
        .add::<ScriptRuntime>()
        .add_value(ScalingExecutor(calls))
        .bind::<dyn ScriptExecutor, ScalingExecutor>();
    catalog
}

#[test]
fn replacement_compiles_validates_and_executes_without_builtin_engines() -> Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = catalog(calls.clone()).build().get_one::<ScriptRuntime>()?;
    let prepared = runtime.prepare(candidate())?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        prepared
            .invocation(
                ScriptContext::default(),
                vec!["/v1/math/scale".into()],
                Default::default(),
                None
            )
            .invoke("/v1/math/scale", &json!(7))?,
        json!(21)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut invalid = candidate();
    invalid.enhancements[0].examples[0].output = json!(99);
    assert!(
        runtime.prepare(invalid).is_err(),
        "替代执行器仍必须通过契约验证"
    );
    let oversized = json!("x".repeat(crate::MAX_VALUE_BYTES));
    let error = prepared
        .invocation(
            ScriptContext::default(),
            vec!["/v1/math/scale".into()],
            Default::default(),
            None,
        )
        .invoke("/v1/math/scale", &oversized)
        .err()
        .context("超额输入未被拒绝")?;
    assert!(error.to_string().contains("输入超过配额"));
    Ok(())
}

#[test]
fn missing_or_conflicting_executor_never_falls_back_to_builtin() -> Result<()> {
    let empty = dill::Catalog::builder()
        .add::<ScriptRuntime>()
        .build()
        .get_one::<ScriptRuntime>()?;
    assert!(empty.validate().is_err());
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = catalog(calls.clone()).build().get_one::<ScriptRuntime>()?;
    let mut typescript = candidate();
    typescript.enhancements[0].language = Language::TypeScript;
    assert!(runtime.prepare(typescript).is_err());
    let runtime = catalog(calls.clone())
        .add_value(ConflictingExecutor)
        .bind::<dyn ScriptExecutor, ConflictingExecutor>()
        .build()
        .get_one::<ScriptRuntime>()?;
    assert!(runtime.prepare(candidate()).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn duplicate_native_type_is_rejected() -> Result<()> {
    let mut catalog = catalog(Arc::new(AtomicUsize::new(0)));
    catalog.bind::<dyn ScriptExecutor, ScalingExecutor>();
    let runtime = catalog.build().get_one::<ScriptRuntime>()?;
    assert!(runtime.validate().is_err());
    Ok(())
}

#[test]
fn empty_dependencies_preserve_existing_persisted_revision_digest() -> Result<()> {
    use sha2::{Digest, Sha256};
    let encoded = r#"{"schema_version":1,"enhancements":[{"path":"/v1/math/scale","language":"rhai","source":"3","examples":[{"input":2,"output":6}]}]}"#;
    let runtime = catalog(Arc::new(AtomicUsize::new(0)))
        .build()
        .get_one::<ScriptRuntime>()?;
    let prepared = runtime.prepare(serde_json::from_str(encoded)?)?;
    assert_eq!(
        prepared.revision(),
        format!("{:x}", Sha256::digest(encoded))
    );
    assert_eq!(serde_json::to_string(prepared.package())?, encoded);
    Ok(())
}
