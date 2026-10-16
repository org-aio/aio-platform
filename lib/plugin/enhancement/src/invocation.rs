use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{PreparedPackage, ScriptContext};

pub(crate) const MAX_DEPTH: usize = 8;
const MAX_CALLS: usize = 64;
const MAX_TRANSFER_BYTES: usize = 1024 * 1024;

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
    deadline: Instant,
    remaining_calls: AtomicUsize,
    remaining_bytes: AtomicUsize,
    cancelled: AtomicBool,
    exhausted: AtomicBool,
}

impl ScriptInvocation {
    pub(crate) fn new(
        package: PreparedPackage,
        context: ScriptContext,
        grants: Vec<String>,
    ) -> Self {
        Self {
            execution: Arc::new(Execution {
                package,
                context,
                grants,
                deadline: Instant::now() + crate::EXECUTION_TIMEOUT,
                remaining_calls: AtomicUsize::new(MAX_CALLS),
                remaining_bytes: AtomicUsize::new(MAX_TRANSFER_BYTES),
                cancelled: AtomicBool::new(false),
                exhausted: AtomicBool::new(false),
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
        self.execution.cancelled.store(true, Ordering::Relaxed);
    }

    /// 执行器的中断检查；预算耗尽后即使脚本捕获异常也不能恢复执行资格。
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.execution.cancelled.load(Ordering::Relaxed),
            "增强调用已取消"
        );
        ensure!(
            !self.execution.exhausted.load(Ordering::Relaxed),
            "增强调用链预算已耗尽"
        );
        ensure!(
            Instant::now() < self.execution.deadline,
            "增强调用链执行超时"
        );
        Ok(())
    }

    /// 执行同一版本中的能力；调用前验证完整依赖权限，执行中逐条约束声明与预算。
    pub fn invoke(&self, path: &str, input: &Value) -> Result<Value> {
        self.check()?;
        self.consume(&self.execution.remaining_calls, 1)?;
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

    fn check_value(&self, value: &Value, direction: &str) -> Result<()> {
        let bytes = serde_json::to_vec(value)?.len();
        ensure!(bytes <= crate::MAX_VALUE_BYTES, "增强{direction}超过配额");
        self.consume(&self.execution.remaining_bytes, bytes)
    }

    fn consume(&self, remaining: &AtomicUsize, amount: usize) -> Result<()> {
        if remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(amount)
            })
            .is_err()
        {
            self.execution.exhausted.store(true, Ordering::Relaxed);
            anyhow::bail!("增强调用链预算已耗尽");
        }
        Ok(())
    }
}
