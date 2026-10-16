//! 释放当前宿主的执行资源，保留安装记录、授权与活动版本供下次恢复。

use anyhow::Result;

use super::Components;

impl Components {
    pub(in crate::runtime::server) async fn shutdown(&self) -> Result<()> {
        let _mutation = self.mutations.lock().await;
        let execution = self.executors.shutdown().await;
        self.active.write().await.clear();
        self.development.write().await.clear();
        self.provisioner.close().await;
        execution
    }
}
