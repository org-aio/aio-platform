use anyhow::{Context, Result};
use serde_json::json;

use crate::{Package, ScriptContext, default_runtime};

fn package() -> Result<Package> {
    Ok(serde_json::from_str(include_str!(
        "../examples/composition.json"
    ))?)
}

#[test]
fn mixed_language_chain_requires_full_grants_and_preserves_version() -> Result<()> {
    let old = default_runtime()?.prepare(package()?)?;
    let grants = old.required_paths("/v1/math/pipeline")?;
    let mut replacement = package()?;
    replacement.enhancements[0].source =
        "fn invoke(input, context) { if input == 3 { 6 } else { input * 3 } }".into();
    let new = default_runtime()?.prepare(replacement)?;
    new.check_examples(old.package())?;
    assert_ne!(old.revision(), new.revision());
    let invocation = old.invocation(ScriptContext::default(), grants.clone(), Default::default());
    assert_eq!(
        invocation.invoke("/v1/math/pipeline", &json!(21))?,
        json!(43)
    );
    assert_eq!(
        new.invocation(ScriptContext::default(), grants, Default::default())
            .invoke("/v1/math/pipeline", &json!(21))?,
        json!(64)
    );
    let limited = old.invocation(
        ScriptContext::default(),
        vec!["/v1/math/pipeline".into(), "/v1/math/double".into()],
        Default::default(),
    );
    let error = limited
        .invoke("/v1/math/pipeline", &json!(21))
        .err()
        .context("未拒绝间接越权")?;
    assert!(error.to_string().contains("未获增强能力: /v1/math/base"));
    Ok(())
}

#[test]
fn candidate_rejects_missing_duplicate_cyclic_and_too_deep_dependencies() -> Result<()> {
    for target in ["/v1/missing/path", "/v1/math/pipeline"] {
        let mut package = package()?;
        package.enhancements[0].calls = vec![target.into()];
        assert!(default_runtime()?.prepare(package).is_err());
    }
    let mut duplicate = package()?;
    duplicate.enhancements[1].calls.push("/v1/math/base".into());
    assert!(default_runtime()?.prepare(duplicate).is_err());
    // 先访问短路径，再访问汇合后的长路径，仍必须校验完整深度。
    let mut deep = package()?;
    let leaf = deep.enhancements[0].clone();
    deep.enhancements.clear();
    for index in 0..9 {
        let mut item = leaf.clone();
        item.path = format!("/v1/chain/node{index}");
        if index > 0 {
            item.calls = vec![format!("/v1/chain/node{}", index - 1)];
        }
        deep.enhancements.push(item);
    }
    let error = default_runtime()?
        .prepare(deep)
        .err()
        .context("深依赖未被拒绝")?;
    assert!(error.to_string().contains("深度配额"));
    Ok(())
}

#[test]
fn dynamic_calls_still_require_declarations_in_both_languages() -> Result<()> {
    for index in [1, 2] {
        let mut package = package()?;
        package.enhancements[index].calls.clear();
        let error = default_runtime()?
            .prepare(package)
            .err()
            .context("未声明依赖仍能执行")?;
        assert!(format!("{error:#}").contains("未声明调用依赖"), "{error:#}");
    }
    Ok(())
}

#[test]
fn nested_scope_cannot_be_forged_by_mutating_script_context() -> Result<()> {
    let package: Package = serde_json::from_value(json!({
        "schema_version": 1,
        "enhancements": [
            {"path":"/v1/context/leaf", "language":"rhai", "source":"fn invoke(input, context) { context }", "examples":[{"input":null,"output":{"tenant_id":"validation","source_id":"validation"}}]},
            {"path":"/v1/context/parent", "language":"typescript", "calls":["/v1/context/leaf"], "source":"export function invoke(input, context) { context.tenant_id = 'forged'; context.source_id = 'forged'; return call_enhancement('/v1/context/leaf', input); }", "examples":[{"input":null,"output":{"tenant_id":"validation","source_id":"validation"}}]}
        ]
    }))?;
    let prepared = default_runtime()?.prepare(package)?;
    for tenant in ["one", "two"] {
        let invocation = prepared.invocation(
            ScriptContext {
                tenant_id: tenant.into(),
                source_id: "plugin".into(),
            },
            prepared.required_paths("/v1/context/parent")?,
            Default::default(),
        );
        assert_eq!(
            invocation.invoke("/v1/context/parent", &json!(null))?,
            json!({"tenant_id":tenant,"source_id":"plugin"})
        );
    }
    Ok(())
}
