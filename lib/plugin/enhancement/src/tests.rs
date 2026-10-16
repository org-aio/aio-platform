use anyhow::Result;
use serde_json::json;

use crate::{Enhancement, Example, Language, Package, ScriptContext, default_runtime};

fn package(language: Language, source: &str) -> Package {
    Package {
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
    }
}

#[test]
fn both_languages_execute_the_same_contract() -> Result<()> {
    for (language, source) in [
        (Language::Rhai, "fn invoke(input, context) { input * 2 }"),
        (
            Language::TypeScript,
            "export function invoke(input: number): number { return input * 2; }",
        ),
    ] {
        let prepared = default_runtime()?.prepare(package(language, source))?;
        assert_eq!(
            prepared.invoke("/v1/math/double", &json!(9), &ScriptContext::default())?,
            json!(18)
        );
        assert!(
            prepared
                .invoke("/v2/math/double", &json!(9), &ScriptContext::default())
                .is_err()
        );
        assert_eq!(prepared.revision().len(), 64);
    }
    Ok(())
}

#[test]
fn upgrade_replays_old_examples_across_languages() -> Result<()> {
    let old = package(Language::Rhai, "fn invoke(input, context) { input * 2 }");
    let new = default_runtime()?.prepare(package(
        Language::TypeScript,
        "export function invoke(input: number) { return input * 2; }",
    ))?;
    new.check_examples(&old)?;
    let mut broken = package(Language::Rhai, "fn invoke(input, context) { input * 3 }");
    broken.enhancements[0].examples[0].output = json!(9);
    let broken = default_runtime()?.prepare(broken)?;
    assert!(broken.check_examples(&old).is_err());
    Ok(())
}

#[test]
fn rejects_failed_candidates_duplicate_paths_and_unversioned_contracts() -> Result<()> {
    let original = package(Language::Rhai, "fn invoke(input, context) { input * 2 }");
    let mut duplicate = original.clone();
    duplicate
        .enhancements
        .push(duplicate.enhancements[0].clone());
    assert!(default_runtime()?.prepare(duplicate).is_err());
    let mut invalid = original.clone();
    invalid.enhancements[0].path = "/math/double".into();
    assert!(default_runtime()?.prepare(invalid).is_err());
    let mut invalid = original.clone();
    invalid.enhancements[0].examples.clear();
    assert!(default_runtime()?.prepare(invalid).is_err());
    let mut invalid = original;
    invalid.enhancements[0].examples[0].output = json!(42);
    assert!(default_runtime()?.prepare(invalid).is_err());
    Ok(())
}

#[test]
fn execution_cannot_load_modules_or_run_forever() -> Result<()> {
    for (language, source) in [
        (Language::Rhai, "fn invoke(input, context) { loop {} }"),
        (
            Language::Rhai,
            "import \"secret\" as secret; fn invoke(input, context) { 6 }",
        ),
        (Language::Rhai, "fn invoke(input, context) { eval(\"6\") }"),
        (
            Language::TypeScript,
            "export function invoke() { while (true) {} }",
        ),
        (
            Language::TypeScript,
            "import fs from 'node:fs'; export function invoke() { return 6; }",
        ),
        (
            Language::TypeScript,
            "export function invoke() { return process.env; }",
        ),
        (
            Language::TypeScript,
            "export async function invoke() { return 6; }",
        ),
    ] {
        assert!(
            default_runtime()?
                .prepare(package(language, source))
                .is_err(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn script_globals_and_context_do_not_leak_between_calls() -> Result<()> {
    let source = "let calls = 0; export function invoke(input: number, context: { tenant_id: string }) { calls++; return { calls, tenant: context.tenant_id }; }";
    let mut candidate = package(Language::TypeScript, source);
    candidate.enhancements[0].examples[0].output = json!({"calls":1,"tenant":"validation"});
    let prepared = default_runtime()?.prepare(candidate)?;
    for tenant in ["first", "second", "first"] {
        let context = ScriptContext {
            tenant_id: tenant.into(),
            source_id: "plugin".into(),
        };
        assert_eq!(
            prepared.invoke("/v1/math/double", &json!(3), &context)?,
            json!({"calls":1,"tenant":tenant})
        );
    }
    Ok(())
}

#[test]
fn documented_package_is_executable() -> Result<()> {
    let package = serde_json::from_str(include_str!("../examples/dual-language.json"))?;
    default_runtime()?.prepare(package)?;
    Ok(())
}

#[test]
fn scripts_cannot_exceed_container_or_js_memory_limits() -> Result<()> {
    for (language, source) in [
        (
            Language::Rhai,
            "fn invoke(input, context) { let value = [0]; for i in 0..30 { value = [value, value]; } 6 }",
        ),
        (
            Language::TypeScript,
            "export function invoke() { return 'x'.repeat(32 * 1024 * 1024); }",
        ),
    ] {
        assert!(
            default_runtime()?
                .prepare(package(language, source))
                .is_err()
        );
    }
    Ok(())
}
