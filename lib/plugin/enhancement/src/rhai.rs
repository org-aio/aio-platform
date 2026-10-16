use anyhow::{Context, Result};
use rhai::{AST, Dynamic, Engine, Scope};
use serde_json::Value;

use crate::{Language, ScriptContext, ScriptExecutor, ScriptProgram};
use std::sync::Arc;

/// 默认受限 Rhai 执行器；由宿主装配，未选择它时不接受 Rhai 候选。
#[dill::component]
#[dill::interface(dyn ScriptExecutor)]
#[dill::scope(dill::Singleton)]
pub struct RhaiExecutor;

impl ScriptExecutor for RhaiExecutor {
    fn supports(&self, language: Language) -> bool {
        language == Language::Rhai
    }
    fn compile(&self, source: &str) -> Result<Arc<dyn ScriptProgram>> {
        Ok(Arc::new(RhaiProgram(compile(source)?)))
    }
}

struct RhaiProgram(AST);

impl ScriptProgram for RhaiProgram {
    fn invoke(&self, input: &Value, context: &ScriptContext) -> Result<Value> {
        invoke(&self.0, input, context)
    }
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(100_000);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_variables(128);
    engine.set_max_functions(64);
    engine.set_max_string_size(crate::MAX_VALUE_BYTES);
    engine.set_max_array_size(4096);
    engine.set_max_map_size(256);
    engine.disable_symbol("eval");
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    let deadline = std::time::Instant::now() + crate::EXECUTION_TIMEOUT;
    engine.on_progress(move |_| {
        (std::time::Instant::now() >= deadline).then(|| Dynamic::from("增强执行超时"))
    });
    engine
}

pub(super) fn compile(source: &str) -> Result<AST> {
    engine().compile(source).context("Rhai 增强编译失败")
}

pub(super) fn invoke(ast: &AST, input: &Value, context: &ScriptContext) -> Result<Value> {
    let input = rhai::serde::to_dynamic(input)?;
    let context = rhai::serde::to_dynamic(context)?;
    let output: Dynamic = engine()
        .call_fn(&mut Scope::new(), ast, "invoke", (input, context))
        .context("Rhai 增强执行失败")?;
    rhai::serde::from_dynamic(&output).context("Rhai 增强必须返回 JSON 值")
}
