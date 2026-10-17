//! 安装意图持久化，执行器可用性来自当前代的服务装配。

use super::{
    RuntimeState,
    http_error::RuntimeError,
    plugins::{ProcessExecutor, WasmExecutor},
};
use crate::runtime::{PluginRuntime, PluginState};
use anyhow::Result;

pub(super) struct ExecutionAvailability {
    wasm: bool,
    process: bool,
}

impl ExecutionAvailability {
    pub fn supports(&self, runtime: PluginRuntime) -> bool {
        match runtime {
            PluginRuntime::WasmComponent => self.wasm,
            PluginRuntime::Process => self.process,
            _ => true,
        }
    }

    pub fn state(&self, runtime: PluginRuntime, enabled: bool) -> PluginState {
        if !enabled {
            PluginState::Disabled
        } else if self.supports(runtime) {
            PluginState::Active
        } else {
            PluginState::Suspended
        }
    }
}

impl RuntimeState {
    pub(super) fn execution_availability(&self) -> Result<ExecutionAvailability> {
        Ok(ExecutionAvailability {
            wasm: self.optional_service::<dyn WasmExecutor>()?.is_some(),
            process: self.optional_service::<dyn ProcessExecutor>()?.is_some(),
        })
    }

    pub(super) fn require_execution(&self, runtime: PluginRuntime) -> Result<(), RuntimeError> {
        if !self.execution_availability()?.supports(runtime) {
            return Err(RuntimeError::unavailable("插件运行环境已暂停"));
        }
        Ok(())
    }
}
