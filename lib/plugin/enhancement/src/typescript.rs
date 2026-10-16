use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};
use rquickjs::{CatchResultExt, Context, Ctx, Exception, Function, Module, Runtime};
use serde_json::Value;

use crate::{Language, ScriptExecutor, ScriptInvocation, ScriptProgram};
use std::sync::Arc;

/// 默认 Oxc 与 QuickJS 执行器；编译产物只缓存代码，每次执行创建独立 JS 上下文。
#[dill::component]
#[dill::interface(dyn ScriptExecutor)]
#[dill::scope(dill::Singleton)]
pub struct TypeScriptExecutor;

impl ScriptExecutor for TypeScriptExecutor {
    fn supports(&self, language: Language) -> bool {
        language == Language::TypeScript
    }
    fn compile(&self, source: &str) -> Result<Arc<dyn ScriptProgram>> {
        Ok(Arc::new(TypeScriptProgram(compile(source)?)))
    }
}

struct TypeScriptProgram(String);

impl ScriptProgram for TypeScriptProgram {
    fn invoke(&self, input: &Value, invocation: &ScriptInvocation) -> Result<Value> {
        invoke(&self.0, input, invocation)
    }
}

pub(super) fn compile(source: &str) -> Result<String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "TypeScript 语法错误: {:?}",
        parsed.diagnostics
    );
    let mut program = parsed.program;
    for statement in &program.body {
        let external = matches!(
            statement,
            Statement::ImportDeclaration(_)
                | Statement::ExportAllDeclaration(_)
                | Statement::ExportFromDeclaration(_)
        );
        ensure!(!external, "TypeScript 增强不允许导入外部模块");
    }
    let semantic = SemanticBuilder::new().with_enum_eval(true).build(&program);
    ensure!(
        semantic.diagnostics.is_empty(),
        "TypeScript 声明错误: {:?}",
        semantic.diagnostics
    );
    let options = TransformOptions::default();
    let transformed = Transformer::new(&allocator, Path::new("enhancement.ts"), &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    ensure!(
        transformed.diagnostics.is_empty(),
        "TypeScript 转译失败: {:?}",
        transformed.diagnostics
    );
    Ok(Codegen::new().build(&program).code)
}

pub(super) fn invoke(code: &str, input: &Value, invocation: &ScriptInvocation) -> Result<Value> {
    let runtime = Runtime::new().context("创建增强 JS 引擎失败")?;
    runtime.set_memory_limit(16 * 1024 * 1024);
    runtime.set_max_stack_size(256 * 1024);
    let progress = invocation.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || progress.check().is_err())));
    let realm = Context::full(&runtime)?;
    realm.with(|ctx| {
        // 不安装模块加载器、文件系统、网络或 Node 全局对象。
        install_call(
            &ctx,
            invocation.clone(),
            "call_enhancement",
            ScriptInvocation::invoke,
        )?;
        install_call(
            &ctx,
            invocation.clone(),
            "call_capability",
            ScriptInvocation::call_capability,
        )?;
        let module = Module::declare(ctx.clone(), "enhancement", code)?;
        let (module, evaluated) = module.eval()?;
        evaluated
            .finish::<()>()
            .context("执行 TypeScript 模块失败")?;
        let invoke: Function = module
            .get("invoke")
            .context("TypeScript 必须导出 invoke 函数")?;
        let input = ctx.json_parse(serde_json::to_vec(input)?)?;
        let context = ctx.json_parse(serde_json::to_vec(invocation.context())?)?;
        let output: rquickjs::Value = invoke
            .call((input, context))
            .catch(&ctx)
            .map_err(|error| anyhow::anyhow!("执行 TypeScript 增强失败: {error}"))?;
        ensure!(
            !output.is_promise(),
            "增强必须同步返回 JSON，不能返回 Promise"
        );
        let output = ctx
            .json_stringify(output)?
            .context("增强必须返回 JSON 值")?
            .to_string()?;
        ensure!(output.len() <= crate::MAX_VALUE_BYTES, "增强输出超过配额");
        serde_json::from_str(&output).context("解析 TypeScript 增强结果失败")
    })
}

fn install_call<'js>(
    ctx: &Ctx<'js>,
    invocation: ScriptInvocation,
    name: &str,
    invoke: fn(&ScriptInvocation, &str, &Value) -> Result<Value>,
) -> rquickjs::Result<()> {
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, path: String, input: rquickjs::Value<'js>| {
            let operation = || -> Result<Vec<u8>> {
                invocation.check()?;
                let input = ctx
                    .json_stringify(input)?
                    .context("增强调用输入必须为 JSON")?
                    .to_string()?;
                ensure!(input.len() <= crate::MAX_VALUE_BYTES, "增强输入超过配额");
                let input = serde_json::from_str(&input)?;
                let output = invoke(&invocation, &path, &input)?;
                Ok(serde_json::to_vec(&output)?)
            };
            let output = operation()
                .map_err(|error| Exception::throw_message(&ctx, &format!("{error:#}")))?;
            ctx.json_parse(output)
        },
    )?;
    ctx.globals().set(name, call)
}
