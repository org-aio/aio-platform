use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};
use rquickjs::{Context, Function, Module, Runtime};
use serde_json::Value;

use crate::ScriptContext;

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

pub(super) fn invoke(code: &str, input: &Value, context: &ScriptContext) -> Result<Value> {
    let runtime = Runtime::new().context("创建增强 JS 引擎失败")?;
    runtime.set_memory_limit(16 * 1024 * 1024);
    runtime.set_max_stack_size(256 * 1024);
    let deadline = std::time::Instant::now() + crate::EXECUTION_TIMEOUT;
    runtime.set_interrupt_handler(Some(Box::new(move || {
        std::time::Instant::now() >= deadline
    })));
    let realm = Context::full(&runtime)?;
    realm.with(|ctx| {
        // 不安装模块加载器、文件系统、网络或 Node 全局对象。
        let module = Module::declare(ctx.clone(), "enhancement", code)?;
        let (module, evaluated) = module.eval()?;
        evaluated
            .finish::<()>()
            .context("执行 TypeScript 模块失败")?;
        let invoke: Function = module
            .get("invoke")
            .context("TypeScript 必须导出 invoke 函数")?;
        let input = ctx.json_parse(serde_json::to_vec(input)?)?;
        let context = ctx.json_parse(serde_json::to_vec(context)?)?;
        let output: rquickjs::Value = invoke
            .call((input, context))
            .context("执行 TypeScript 增强失败")?;
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
