use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{CapabilityCaller, InvocationCapabilities, InvocationControl, Package, ScriptContext};

const ROOT: &str = "/v1/example/root";
const CHILD: &str = "/v1/example/child";
const SERVICE: &str = "/v1/service/echo";

#[test]
fn documented_worker_composition_validates_without_connecting_to_a_host() -> Result<()> {
    crate::default_runtime()?.prepare(serde_json::from_str(include_str!(
        "../examples/worker-list.json"
    ))?)?;
    Ok(())
}

struct Echo {
    calls: AtomicUsize,
    delay: std::time::Duration,
}
impl CapabilityCaller for Echo {
    fn available(&self, path: &str) -> bool {
        path == SERVICE
    }
    fn call(&self, _: &str, input: &Value, control: &InvocationControl) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + self.delay;
        while std::time::Instant::now() < deadline {
            control.check()?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(input.clone())
    }
}

fn package(typescript_first: bool) -> Result<Package> {
    let root = if typescript_first {
        (
            "typescript",
            "export function invoke(input, context) { context.tenant_id = 'forged'; return call_enhancement('/v1/example/child', input); }",
        )
    } else {
        (
            "rhai",
            "fn invoke(input, context) { context.tenant_id = \"forged\"; call_enhancement(\"/v1/example/child\", input) }",
        )
    };
    let child = if typescript_first {
        (
            "rhai",
            "fn invoke(input, context) { call_capability(\"/v1/service/echo\", #{tenant:context.tenant_id,value:input}) }",
        )
    } else {
        (
            "typescript",
            "export function invoke(input, context) { return call_capability('/v1/service/echo', {tenant:context.tenant_id,value:input}); }",
        )
    };
    let example = json!({"input":7,"output":{"tenant":"validation","value":7},"capabilities":[{"path":SERVICE,"input":{"tenant":"validation","value":7},"output":{"tenant":"validation","value":7}}]});
    Ok(serde_json::from_value(
        json!({"schema_version":1,"enhancements":[
            {"path":ROOT,"language":root.0,"source":root.1,"calls":[CHILD],"examples":[example]},
            {"path":CHILD,"language":child.0,"source":child.1,"capabilities":[SERVICE],"examples":[example]}
        ]}),
    )?)
}

#[test]
fn both_languages_compose_host_calls_with_transitive_grants_and_immutable_identity() -> Result<()> {
    for typescript_first in [false, true] {
        let prepared = crate::default_runtime()?.prepare(package(typescript_first)?)?;
        let caller = Arc::new(Echo {
            calls: AtomicUsize::new(0),
            delay: std::time::Duration::ZERO,
        });
        let context = ScriptContext {
            tenant_id: "tenant-a".into(),
            source_id: "plugin".into(),
        };
        let grants = vec![ROOT.into(), CHILD.into()];
        let denied = prepared.invocation(
            context.clone(),
            grants.clone(),
            InvocationCapabilities {
                grants: vec![],
                caller: Some(caller.clone()),
            },
        );
        assert!(denied.invoke(ROOT, &json!(42)).is_err());
        assert_eq!(caller.calls.load(Ordering::SeqCst), 0);
        let invocation = prepared.invocation(
            context,
            grants,
            InvocationCapabilities {
                grants: vec![SERVICE.into()],
                caller: Some(caller.clone()),
            },
        );
        assert_eq!(
            invocation.invoke(ROOT, &json!(42))?,
            json!({"tenant":"tenant-a","value":42})
        );
        assert_eq!(caller.calls.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
fn candidate_validation_requires_exact_and_fully_consumed_capability_transcripts() -> Result<()> {
    let runtime = crate::default_runtime()?;
    for invalid in 0..4 {
        let mut candidate = package(false)?;
        let calls = &mut candidate.enhancements[0].examples[0].capabilities;
        match invalid {
            0 => calls.clear(),
            1 => calls[0].input = json!("unexpected"),
            2 => calls.push(calls[0].clone()),
            _ => candidate.enhancements[1].capabilities.clear(),
        }
        assert!(
            runtime.prepare(candidate).is_err(),
            "invalid transcript {invalid}"
        );
    }
    Ok(())
}

#[test]
fn catching_transcript_mismatches_cannot_validate_extra_or_reordered_calls() -> Result<()> {
    let runtime = crate::default_runtime()?;
    for (language, source) in [
        (
            "typescript",
            "export function invoke(input) { call_capability('/v1/service/echo', input); try { call_capability('/v1/service/echo', input); } catch(_) {} return input; }",
        ),
        (
            "typescript",
            "export function invoke(input) { try { call_capability('/v1/service/echo', 999); } catch(_) {} return call_capability('/v1/service/echo', input); }",
        ),
        (
            "rhai",
            "fn invoke(input, context) { call_capability(\"/v1/service/echo\", input); try { call_capability(\"/v1/service/echo\", input); } catch (error) {} input }",
        ),
        (
            "rhai",
            "fn invoke(input, context) { try { call_capability(\"/v1/service/echo\", 999); } catch (error) {} call_capability(\"/v1/service/echo\", input) }",
        ),
    ] {
        let package: Package = serde_json::from_value(json!({"schema_version":1,"enhancements":[{
            "path":ROOT,"language":language,"source":source,"capabilities":[SERVICE],
            "examples":[{"input":7,"output":7,"capabilities":[{"path":SERVICE,"input":7,"output":7}]}]
        }]}))?;
        let error = runtime
            .prepare(package)
            .err()
            .context("捕获异常掩盖了调用记录不匹配")?;
        assert!(
            format!("{error:#}").contains("不匹配"),
            "{language}: {error:#}"
        );
    }
    Ok(())
}

#[test]
fn host_waiting_preserves_compute_budget_and_cancellation() -> Result<()> {
    let prepared = crate::default_runtime()?.prepare(package(true)?)?;
    let caller = Arc::new(Echo {
        calls: AtomicUsize::new(0),
        delay: std::time::Duration::from_millis(150),
    });
    let invocation = prepared.invocation(
        ScriptContext::default(),
        vec![ROOT.into(), CHILD.into()],
        InvocationCapabilities {
            grants: vec![SERVICE.into()],
            caller: Some(caller.clone()),
        },
    );
    assert_eq!(
        invocation.invoke(ROOT, &json!(1))?,
        json!({"tenant":"","value":1})
    );
    let invocation = prepared.invocation(
        ScriptContext::default(),
        vec![ROOT.into(), CHILD.into()],
        InvocationCapabilities {
            grants: vec![SERVICE.into()],
            caller: Some(caller.clone()),
        },
    );
    let worker = invocation.clone();
    let thread = std::thread::spawn(move || worker.invoke(ROOT, &json!(1)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while caller.calls.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    invocation.cancel();
    let error = thread
        .join()
        .map_err(|_| anyhow::anyhow!("测试调用线程 panic"))?
        .err()
        .context("取消未生效")?;
    assert!(format!("{error:#}").contains("已取消"), "{error:#}");
    Ok(())
}

#[test]
fn catching_host_errors_cannot_reset_shared_call_or_transfer_limits() -> Result<()> {
    for (language, source) in [
        (
            "typescript",
            "export function invoke(input) { for(let i=0;i<(input ? 4:1);i++){ try {call_capability('/v1/service/echo', null);} catch(_){} } return input; }",
        ),
        (
            "rhai",
            "fn invoke(input, context) { let count = if input { 4 } else { 1 }; for i in 0..count { try { call_capability(\"/v1/service/echo\", ()); } catch (error) {} } input }",
        ),
    ] {
        let package: Package = serde_json::from_value(json!({"schema_version":1,"enhancements":[{
            "path":ROOT,"language":language,"source":source,
            "capabilities":[SERVICE],"examples":[{"input":false,"output":false,"capabilities":[{"path":SERVICE,"input":null,"output":null}]}]
        }]}))?;
        let prepared = crate::default_runtime()?.prepare(package)?;
        for transfer in [false, true] {
            let caller = Arc::new(NearlyExhausted {
                calls: AtomicUsize::new(0),
                transfer,
            });
            let invocation = prepared.invocation(
                ScriptContext::default(),
                vec![ROOT.into()],
                InvocationCapabilities {
                    grants: vec![SERVICE.into()],
                    caller: Some(caller.clone()),
                },
            );
            let error = invocation
                .invoke(ROOT, &json!(true))
                .err()
                .context("宿主调用未受共享配额限制")?;
            assert!(format!("{error:#}").contains("预算已耗尽"), "{error:#}");
            assert_eq!(caller.calls.load(Ordering::SeqCst), 2);
            assert!(invocation.check().is_err());
            assert!(invocation.invoke(ROOT, &json!(false)).is_err());
            assert_eq!(caller.calls.load(Ordering::SeqCst), 2);
        }
    }
    Ok(())
}

// 先消耗同一调用链的大部分配额，再由真实脚本越界，避免大量 JSON 转换与 100ms 超时竞争。
struct NearlyExhausted {
    calls: AtomicUsize,
    transfer: bool,
}

impl CapabilityCaller for NearlyExhausted {
    fn available(&self, path: &str) -> bool {
        path == SERVICE
    }

    fn call(&self, _: &str, input: &Value, control: &InvocationControl) -> Result<Value> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            if self.transfer {
                control.transfer(1024 * 1024 - 16)?;
            } else {
                for _ in 0..61 {
                    control.call()?;
                }
            }
        }
        Ok(input.clone())
    }
}
