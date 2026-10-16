//! 释放当前宿主的执行资源，保留安装记录、授权与活动版本供下次恢复。

use anyhow::Result;

use super::Components;

impl Components {
    pub(in crate::runtime::server) async fn shutdown(&self) -> Result<()> {
        let _mutation = self.mutations.lock().await;
        let processes = self.processes.shutdown().await;
        let slots = std::mem::take(&mut *self.slots.lock().await);
        for slot in slots.into_values() {
            slot.unload().await;
        }
        let development = std::mem::take(&mut *self.development.write().await);
        for instance in development.into_values() {
            if let Some(slot) = instance.slot {
                slot.deactivate().await;
            }
        }
        self.provisioner.close().await;
        processes
    }
}
