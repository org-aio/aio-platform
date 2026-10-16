//! 释放当前宿主的执行资源，保留安装记录、授权与活动版本供下次恢复。

use anyhow::{Result, ensure};

use super::Components;

impl Components {
    pub(in crate::runtime::server) async fn shutdown(&self) -> Result<()> {
        let _mutation = self.mutations.lock().await;
        let execution = self.executors.shutdown().await;
        self.active.write().await.clear();
        self.development.write().await.clear();
        let resources = self.backends.shutdown().await;
        let errors: Vec<_> = [execution, resources]
            .into_iter()
            .filter_map(Result::err)
            .map(|error| format!("{error:#}"))
            .collect();
        ensure!(errors.is_empty(), "关闭组件环境失败: {}", errors.join("; "));
        Ok(())
    }
}
