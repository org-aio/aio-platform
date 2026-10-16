use std::{any::Any, collections::HashSet, sync::Arc};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::{Language, ScriptContext};

/// 原生执行器扩展点；由 Dill 按具体类型注册，语言是输入格式而非插件身份。
/// 实现必须隔离调用状态并限制执行资源，宿主仍统一校验包、授权与输入输出配额。
pub trait ScriptExecutor: Any + Send + Sync {
    fn supports(&self, language: Language) -> bool;
    fn compile(&self, source: &str) -> Result<Arc<dyn ScriptProgram>>;
}

/// 已编译的纯计算程序；可以并发调用，每次调用独立的输入和可信归属由宿主提供。
pub trait ScriptProgram: Any + Send + Sync {
    fn invoke(&self, input: &Value, context: &ScriptContext) -> Result<Value>;
}

/// 从同一个 Dill 容器解析执行器，所有候选验证、版本恢复与在线执行使用此实例。
#[dill::component]
#[dill::scope(dill::Singleton)]
pub struct ScriptRuntime {
    executors: Vec<Arc<dyn ScriptExecutor>>,
}

impl ScriptRuntime {
    /// 启动时拒绝空装配或同一具体类型的重复绑定，不按注册先后覆盖实现。
    pub fn validate(&self) -> Result<()> {
        let mut identities = HashSet::new();
        for executor in self.executors.iter() {
            ensure!(
                identities.insert(executor.as_ref().type_id()),
                "脚本执行器类型被重复注册"
            );
        }
        ensure!(!identities.is_empty(), "未装配任何脚本执行器");
        Ok(())
    }

    pub(crate) fn compile(
        &self,
        language: Language,
        source: &str,
    ) -> Result<Arc<dyn ScriptProgram>> {
        let mut executors = self
            .executors
            .iter()
            .filter(|executor| executor.supports(language));
        let executor = executors.next().context("未装配支持该语言的脚本执行器")?;
        ensure!(
            executors.next().is_none(),
            "同一语言存在多个脚本执行器，必须明确选择实现"
        );
        executor.compile(source)
    }
}

/// 独立验证工具的默认组合；嵌入宿主应在自己的 Dill 容器中选择执行器。
pub fn default_runtime() -> Result<Arc<ScriptRuntime>> {
    let mut catalog = dill::CatalogBuilder::new();
    catalog.add::<ScriptRuntime>();
    #[cfg(feature = "rhai")]
    catalog.add::<crate::RhaiExecutor>();
    #[cfg(feature = "typescript")]
    catalog.add::<crate::TypeScriptExecutor>();
    catalog.validate().context("校验脚本执行器依赖失败")?;
    let runtime = catalog.build().get_one::<ScriptRuntime>()?;
    runtime.validate()?;
    Ok(runtime)
}
