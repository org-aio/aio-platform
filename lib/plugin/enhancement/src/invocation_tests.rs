use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{
    Language, Package, ScriptContext, ScriptExecutor, ScriptInvocation, ScriptProgram,
    ScriptRuntime,
};

// 使用原生测试执行器精确制造阻塞与预算消耗，避免依赖解释器速度来断言超时。
struct Executor(mpsc::Sender<()>);
struct Program {
    source: String,
    entered: mpsc::Sender<()>,
}

impl ScriptExecutor for Executor {
    fn supports(&self, language: Language) -> bool {
        language == Language::Rhai
    }
    fn compile(&self, source: &str) -> Result<Arc<dyn ScriptProgram>> {
        Ok(Arc::new(Program {
            source: source.into(),
            entered: self.0.clone(),
        }))
    }
}

impl ScriptProgram for Program {
    fn invoke(&self, input: &Value, invocation: &ScriptInvocation) -> Result<Value> {
        match self.source.as_str() {
            "leaf" => {
                if let Some(delay) = input.get("delay_ms").and_then(Value::as_u64) {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                Ok(input.clone())
            }
            "repeat" => {
                let count = input["count"].as_u64().context("缺少调用次数")?;
                for _ in 0..count {
                    let result = invocation.invoke("/v1/test/leaf", &input["payload"]);
                    if input["catch"] != true {
                        result?;
                    }
                }
                Ok(json!(count))
            }
            "recursive" if input == true => invocation.invoke("/v1/test/root", input),
            "block" if input == true => {
                self.entered.send(())?;
                loop {
                    invocation.check()?;
                    std::thread::yield_now();
                }
            }
            _ => Ok(input.clone()),
        }
    }
}

fn prepare(source: &str) -> Result<(ScriptInvocation, mpsc::Receiver<()>)> {
    let (entered, receiver) = mpsc::channel();
    let runtime = dill::Catalog::builder()
        .add::<ScriptRuntime>()
        .add_value(Executor(entered))
        .bind::<dyn ScriptExecutor, Executor>()
        .build()
        .get_one::<ScriptRuntime>()?;
    let (calls, input, output) = if source == "repeat" {
        (
            vec!["/v1/test/leaf"],
            json!({"count":1,"payload":null}),
            json!(1),
        )
    } else {
        (vec![], json!(false), json!(false))
    };
    let package: Package = serde_json::from_value(json!({"schema_version":1,"enhancements":[
        {"path":"/v1/test/leaf","language":"rhai","source":"leaf","examples":[{"input":1,"output":1}]},
        {"path":"/v1/test/root","language":"rhai","source":source,"calls":calls,"examples":[{"input":input,"output":output}]}
    ]}))?;
    let prepared = runtime.prepare(package)?;
    Ok((
        prepared.invocation(
            ScriptContext::default(),
            vec!["/v1/test/root".into(), "/v1/test/leaf".into()],
        ),
        receiver,
    ))
}

#[test]
fn call_and_transfer_budgets_apply_to_the_whole_chain_even_when_errors_are_caught() -> Result<()> {
    for input in [
        json!({"count":100,"payload":null,"catch":true}),
        json!({"count":20,"payload":"x".repeat(60_000),"catch":true}),
    ] {
        let (invocation, _) = prepare("repeat")?;
        let error = invocation
            .invoke("/v1/test/root", &input)
            .err()
            .context("共享预算未生效")?;
        assert!(format!("{error:#}").contains("预算已耗尽"), "{error:#}");
        assert!(invocation.check().is_err());
    }
    Ok(())
}

#[test]
fn nested_calls_share_one_deadline_and_cannot_restart_the_clock() -> Result<()> {
    let (invocation, _) = prepare("repeat")?;
    let started = Instant::now();
    let error = invocation
        .invoke(
            "/v1/test/root",
            &json!({"count":8,"payload":{"delay_ms":30}}),
        )
        .err()
        .context("嵌套调用重置了时限")?;
    assert!(format!("{error:#}").contains("执行超时"), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(1));
    Ok(())
}

#[test]
fn cancellation_reaches_running_native_executor_and_all_clones() -> Result<()> {
    let (invocation, receiver) = prepare("block")?;
    let worker = invocation.clone();
    let thread = std::thread::spawn(move || worker.invoke("/v1/test/root", &json!(true)));
    receiver.recv_timeout(Duration::from_secs(1))?;
    invocation.cancel();
    let error = thread
        .join()
        .map_err(|_| anyhow::anyhow!("测试工作线程 panic"))?
        .err()
        .context("取消后仍成功")?;
    assert!(format!("{error:#}").contains("已取消"), "{error:#}");
    assert!(invocation.invoke("/v1/test/leaf", &json!(1)).is_err());
    Ok(())
}

#[test]
fn native_executor_cannot_recursively_reenter_the_current_capability() -> Result<()> {
    let (invocation, _) = prepare("recursive")?;
    let error = invocation
        .invoke("/v1/test/root", &json!(true))
        .err()
        .context("运行时循环未被拒绝")?;
    assert!(format!("{error:#}").contains("存在循环"));
    Ok(())
}
