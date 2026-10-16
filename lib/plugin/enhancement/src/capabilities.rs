use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::InvocationControl;

/// 宿主按真实调用者绑定的能力入口；脚本不能自行构造提供者或更改授权。
pub trait CapabilityCaller: Any + Send + Sync {
    fn available(&self, path: &str) -> bool;
    fn call(&self, path: &str, input: &Value, control: &InvocationControl) -> Result<Value>;
}

/// 当前请求获准使用的宿主能力；默认拒绝全部，声明依赖不会自动获得授权。
#[derive(Clone, Default)]
pub struct InvocationCapabilities {
    pub grants: Vec<String>,
    pub caller: Option<Arc<dyn CapabilityCaller>>,
}

/// 候选样例中的一次外部调用记录；按顺序精确匹配，不连接真实服务。
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityExample {
    pub path: String,
    pub input: Value,
    pub output: Value,
}

pub(crate) struct ExampleCaller {
    calls: Vec<CapabilityExample>,
    progress: Mutex<TranscriptProgress>,
}

#[derive(Default)]
struct TranscriptProgress {
    cursor: usize,
    failed: bool,
}

impl ExampleCaller {
    pub(crate) fn new(calls: Vec<CapabilityExample>) -> Self {
        Self {
            calls,
            progress: Mutex::default(),
        }
    }

    pub(crate) fn finish(&self) -> Result<()> {
        let progress = self
            .progress
            .lock()
            .map_err(|_| anyhow::anyhow!("样例调用锁损坏"))?;
        ensure!(!progress.failed, "增强样例包含不匹配的宿主调用");
        ensure!(
            progress.cursor == self.calls.len(),
            "增强样例存在未执行的宿主调用"
        );
        Ok(())
    }
}

impl CapabilityCaller for ExampleCaller {
    fn available(&self, _: &str) -> bool {
        true
    }

    fn call(&self, path: &str, input: &Value, control: &InvocationControl) -> Result<Value> {
        control.check()?;
        let mut progress = self
            .progress
            .lock()
            .map_err(|_| anyhow::anyhow!("样例调用锁损坏"))?;
        ensure!(!progress.failed, "增强样例包含不匹配的宿主调用");
        // 调用记录不匹配是验证失败，脚本捕获异常也不能恢复此样例的有效性。
        let Some(expected) = self.calls.get(progress.cursor) else {
            progress.failed = true;
            bail!("增强样例缺少宿主调用响应");
        };
        if expected.path != path || &expected.input != input {
            progress.failed = true;
            bail!("增强样例的宿主调用顺序或输入不匹配: {path}");
        }
        progress.cursor += 1;
        Ok(expected.output.clone())
    }
}
