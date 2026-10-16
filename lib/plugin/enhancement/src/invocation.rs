use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{InvocationCapabilities, InvocationControl, PreparedPackage, ScriptContext};

pub(crate) const MAX_DEPTH: usize = 8;

/// 宿主创建的调用链；克隆只复制调用栈，版本、身份、权限与预算始终共享。
/// 原生执行器必须定期调用 check，子调用只允许经过 invoke。
#[derive(Clone)]
pub struct ScriptInvocation {
    execution: Arc<Execution>,
    stack: Vec<String>,
}

struct Execution {
    package: PreparedPackage,
    context: ScriptContext,
    grants: Vec<String>,
    capabilities: InvocationCapabilities,
    control: InvocationControl,
}

impl ScriptInvocation {
    pub(crate) fn new(
        package: PreparedPackage,
        context: ScriptContext,
        grants: Vec<String>,
        capabilities: InvocationCapabilities,
        deadline: Option<std::time::Instant>,
    ) -> Self {
        Self {
            execution: Arc::new(Execution {
                package,
                context,
                grants,
                capabilities,
                control: InvocationControl::new(deadline),
            }),
            stack: Vec::new(),
        }
    }

    /// 只读可信身份；脚本获得的是此数据的副本，不能改写后续调用的身份。
    pub fn context(&self) -> &ScriptContext {
        &self.execution.context
    }

    /// 取消整条调用链，包括尚未开始的工作线程与正在执行的嵌套脚本。
    pub fn cancel(&self) {
        self.execution.control.cancel();
    }

    /// 执行器的中断检查；预算耗尽后即使脚本捕获异常也不能恢复执行资格。
    pub fn check(&self) -> Result<()> {
        self.execution.control.check()
    }

    /// 执行同一版本中的能力；调用前验证完整依赖权限，执行中逐条约束声明与预算。
    pub fn invoke(&self, path: &str, input: &Value) -> Result<Value> {
        self.check()?;
        self.execution.control.call()?;
        ensure!(self.stack.len() < MAX_DEPTH, "增强调用超过深度配额");
        ensure!(
            !self.stack.iter().any(|entry| entry == path),
            "增强调用存在循环: {path}"
        );
        let package = &self.execution.package;
        if let Some(caller) = self.stack.last() {
            let declaration = package
                .package()
                .enhancements
                .iter()
                .find(|entry| &entry.path == caller)
                .context("调用方增强不存在")?;
            ensure!(
                declaration.calls.iter().any(|entry| entry == path),
                "增强未声明调用依赖: {path}"
            );
        }
        for required in package.required_paths(path)? {
            ensure!(
                self.execution.grants.contains(&required),
                "调用者未获增强能力: {required}"
            );
        }
        for required in package.required_capabilities(path)? {
            ensure!(
                self.execution.capabilities.grants.contains(&required),
                "调用者未获宿主能力: {required}"
            );
            ensure!(
                self.execution
                    .capabilities
                    .caller
                    .as_ref()
                    .is_some_and(|caller| caller.available(&required)),
                "宿主未装配能力: {required}"
            );
        }
        self.check_value(input, "输入")?;
        let implementation = package
            .compiled
            .get(path)
            .context("运行时尚未提供该增强能力")?;
        let mut child = self.clone();
        child.stack.push(path.into());
        self.check()?;
        let result = implementation.invoke(input, &child);
        self.check()?;
        let output = result.with_context(|| format!("执行增强失败: {path}"))?;
        self.check_value(&output, "输出")?;
        Ok(output)
    }

    /// 调用当前增强声明的宿主能力，共享身份、预算和取消；实现只由宿主提供。
    pub fn call_capability(&self, path: &str, input: &Value) -> Result<Value> {
        self.check()?;
        self.execution.control.call()?;
        let source = self
            .stack
            .last()
            .context("宿主能力必须由正在执行的增强调用")?;
        let declaration = self
            .execution
            .package
            .package()
            .enhancements
            .iter()
            .find(|entry| &entry.path == source)
            .context("调用方增强不存在")?;
        ensure!(
            declaration.capabilities.iter().any(|entry| entry == path),
            "增强未声明宿主能力: {path}"
        );
        ensure!(
            self.execution
                .capabilities
                .grants
                .iter()
                .any(|entry| entry == path),
            "调用者未获宿主能力: {path}"
        );
        let caller = self
            .execution
            .capabilities
            .caller
            .as_ref()
            .context("宿主能力调用入口未绑定")?;
        ensure!(caller.available(path), "宿主未装配能力: {path}");
        self.check_value(input, "输入")?;
        let wait = self.execution.control.wait()?;
        let result = caller.call(path, input, &self.execution.control);
        drop(wait);
        self.check()?;
        let output = result.with_context(|| format!("调用宿主能力失败: {path}"))?;
        self.check_value(&output, "输出")?;
        Ok(output)
    }

    fn check_value(&self, value: &Value, direction: &str) -> Result<()> {
        let bytes = serde_json::to_vec(value)?.len();
        ensure!(bytes <= crate::MAX_VALUE_BYTES, "增强{direction}超过配额");
        self.execution.control.transfer(bytes)
    }
}
