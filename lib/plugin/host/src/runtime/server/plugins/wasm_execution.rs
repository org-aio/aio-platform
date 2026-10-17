//! v1 页面执行契约独立于内置 Wasmtime；整包执行使用 ComponentExecutor。

use std::path::Path;

pub use az_plugin_manifest::WasmArtifactValidator;

use crate::runtime::{ComponentResponse, PageDefinition};
use anyhow::Result;

/// 页面实例激活结果；created 决定提交失败时是否需要清理候选。
pub struct WasmActivation {
    pub pages: Vec<PageDefinition>,
    pub created: bool,
}

/// v1 Wasm 页面执行服务；所有入口从 Dill 获取同一个实现。
pub trait WasmExecutor: WasmArtifactValidator {
    fn activate(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        artifact: &Path,
    ) -> Result<WasmActivation>;
    fn deactivate(&self, tenant_id: &str, source_id: &str, revision: &str) -> Result<bool>;
    fn handle(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        request: String,
    ) -> Result<ComponentResponse>;
    fn bind_revision(
        &self,
        tenant: &str,
        source: &str,
        backend: &str,
        revision: &str,
    ) -> Result<()>;
    fn retain_development_revisions(
        &self,
        tenant: &str,
        source: &str,
        backend: &str,
        revision: &str,
    ) -> Result<()>;
    fn shutdown(&self) -> Result<()>;
}
