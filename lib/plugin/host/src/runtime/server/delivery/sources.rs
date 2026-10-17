//! 发现器与构建队列之间的中立契约；仓库地址是业务来源，不是插件身份。
use anyhow::{Result, ensure};
use az_plugin_delivery::BuildRecipe;
use std::any::Any;

/// 一次读取的来源快照；generation 用于拒绝网络请求期间已经过时的观察。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeliverySource {
    pub git: String,
    pub branch: String,
    pub source_revision: String,
    pub enabled: bool,
    pub generation: i64,
}

/// 发现器在固定源码提交上读取的构建配方。
#[derive(Clone, Debug)]
pub struct DeliveryTarget {
    pub branch: String,
    pub source_revision: String,
    pub recipe: BuildRecipe,
}
impl DeliveryTarget {
    /// 正式存储和替代存储共享校验，拒绝不完整 SHA、空分支及无效配方。
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.branch.is_empty() && self.branch.len() <= 1024 && !self.branch.contains('\0'),
            "交付分支无效"
        );
        ensure!(
            self.source_revision.len() == 40
                && self.source_revision.bytes().all(|b| b.is_ascii_hexdigit()),
            "无效提交 SHA"
        );
        az_plugin_delivery::validate_recipe(&self.recipe)
    }
}

/// 由发现、webhook 或本地仓库插件调用；实现必须原子切换目标并取消旧租约。
/// 撤销也要保留代数，避免仍在网络中等待的旧观察重新启用来源。
#[async_trait::async_trait]
pub trait DeliverySources: Any + Send + Sync {
    async fn get(&self, git: &str) -> Result<Option<DeliverySource>>;
    async fn enabled(&self) -> Result<Vec<DeliverySource>>;
    /// 只有当前代数等于 expected 才提交，首次观察使用 None；冲突返回 false。
    /// None 目标表示撤销。提交失败不能留下目标或任务，也不能伪装成代数冲突。
    async fn update(
        &self,
        git: &str,
        expected: Option<i64>,
        target: Option<DeliveryTarget>,
    ) -> Result<bool>;
}
