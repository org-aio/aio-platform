use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    Language, Package, ScriptContext, ScriptExecutor, ScriptInvocation, ScriptProgram,
    ScriptRuntime,
};

fn candidate() -> Result<Package> {
    Ok(serde_json::from_value(
        json!({"schema_version":1,"enhancements":[{
            "path":"/v1/value/echo", "language":"rhai", "source":"identity",
            "input_schema":{"type":"integer","minimum":0},
            "output_schema":{"type":"integer","maximum":5},
            "examples":[{"input":3,"output":3}]
        }]}),
    )?)
}

struct IdentityExecutor(Arc<AtomicUsize>);
struct IdentityProgram(Arc<AtomicUsize>);
impl ScriptExecutor for IdentityExecutor {
    fn supports(&self, _: Language) -> bool {
        true
    }
    fn compile(&self, _: &str) -> Result<Arc<dyn ScriptProgram>> {
        Ok(Arc::new(IdentityProgram(self.0.clone())))
    }
}
impl ScriptProgram for IdentityProgram {
    fn invoke(&self, input: &Value, _: &ScriptInvocation) -> Result<Value> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(input.clone())
    }
}

fn runtime(calls: Arc<AtomicUsize>) -> Result<Arc<ScriptRuntime>> {
    let mut catalog = dill::CatalogBuilder::new();
    catalog
        .add::<ScriptRuntime>()
        .add_value(IdentityExecutor(calls))
        .bind::<dyn ScriptExecutor, IdentityExecutor>();
    Ok(catalog.build().get_one::<ScriptRuntime>()?)
}

#[test]
fn shared_chain_validates_before_replacement_execution_and_before_returning_output() -> Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = runtime(calls.clone())?;
    let prepared = runtime.prepare(candidate()?)?;
    let invoke = |input: Value| {
        prepared
            .invocation(
                ScriptContext::default(),
                vec!["/v1/value/echo".into()],
                Default::default(),
                None,
            )
            .invoke("/v1/value/echo", &input)
    };
    let initial = calls.load(Ordering::SeqCst);
    assert!(invoke(json!("private-input")).is_err());
    assert!(invoke(json!(-1)).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), initial);
    let error = invoke(json!(7)).err().context("输出约束未生效")?;
    assert!(error.to_string().contains("输出不符合 schema"));
    assert_eq!(calls.load(Ordering::SeqCst), initial + 1);
    assert_eq!(invoke(json!(4))?, json!(4));
    let mut invalid_sample = candidate()?;
    invalid_sample.enhancements[0].examples[0].input = json!(-1);
    assert!(runtime.prepare(invalid_sample).is_err());
    Ok(())
}

#[test]
fn schemas_are_immutable_across_languages_and_default_fields_preserve_bytes() -> Result<()> {
    let runtime = runtime(Default::default())?;
    let previous = candidate()?;
    let mut replacement = previous.clone();
    replacement.enhancements[0].language = Language::TypeScript;
    runtime
        .prepare(replacement.clone())?
        .check_examples(&previous)?;
    replacement.enhancements[0].input_schema =
        Some(json!({"type":"integer","minimum":2}).try_into()?);
    assert!(
        runtime
            .prepare(replacement)?
            .check_examples(&previous)
            .is_err()
    );
    let mut relaxed = previous.clone();
    relaxed.enhancements[0].output_schema = None;
    assert!(runtime.prepare(relaxed)?.check_examples(&previous).is_err());
    let mut untyped = candidate()?;
    untyped.enhancements[0].input_schema = None;
    untyped.enhancements[0].output_schema = None;
    let bytes = serde_json::to_string(&untyped)?;
    assert!(!bytes.contains("input_schema") && !bytes.contains("output_schema"));
    assert_eq!(
        untyped.enhancements[0].contract().input_schema.as_value(),
        &json!(true)
    );
    let mut explicit = untyped.clone();
    explicit.enhancements[0].input_schema = Some(true.into());
    runtime.prepare(explicit)?.check_examples(&untyped)?;
    Ok(())
}

#[cfg(all(feature = "rhai", feature = "typescript"))]
#[test]
fn real_languages_and_nested_calls_obey_the_same_schema() -> Result<()> {
    for (language, source) in [
        (Language::Rhai, "fn invoke(input, context) { input }"),
        (
            Language::TypeScript,
            "export function invoke(input: number) { return input; }",
        ),
    ] {
        let mut package = candidate()?;
        package.enhancements[0].language = language;
        package.enhancements[0].source = source.into();
        let prepared = crate::default_runtime()?.prepare(package)?;
        let invoke = |input| {
            prepared
                .invocation(
                    ScriptContext::default(),
                    vec!["/v1/value/echo".into()],
                    Default::default(),
                    None,
                )
                .invoke("/v1/value/echo", &input)
        };
        assert_eq!(invoke(json!(4))?, json!(4));
        assert!(invoke(json!(7)).is_err());
        assert!(invoke(json!("string")).is_err());
    }
    let package: Package = serde_json::from_value(json!({"schema_version":1,"enhancements":[
        {"path":"/v1/value/parent","language":"rhai","source":"fn invoke(input, context) { call_enhancement(\"/v1/value/child\", input) }","calls":["/v1/value/child"],"examples":[{"input":3,"output":3}]},
        {"path":"/v1/value/child","language":"typescript","source":"export function invoke(input: number) { return input; }","input_schema":{"type":"integer"},"output_schema":{"type":"integer"},"examples":[{"input":3,"output":3}]}
    ]}))?;
    let prepared = crate::default_runtime()?.prepare(package)?;
    let invocation = prepared.invocation(
        ScriptContext::default(),
        vec!["/v1/value/parent".into(), "/v1/value/child".into()],
        Default::default(),
        None,
    );
    let error = invocation
        .invoke("/v1/value/parent", &json!("string"))
        .err()
        .context("嵌套输入未检查")?;
    assert!(format!("{error:#}").contains("输入不符合 schema"));
    Ok(())
}
