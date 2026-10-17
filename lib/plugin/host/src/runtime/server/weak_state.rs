//! 内部能力只保存运行时弱引用，避免服务容器经管理入口引用自身。

use std::sync::Weak;

use anyhow::{Context, Result};

use super::{RuntimeState, plugins::RuntimeExtensions};

pub(super) struct WeakRuntimeState {
    extensions: Weak<RuntimeExtensions>,
    restore: Box<dyn Fn(std::sync::Arc<RuntimeExtensions>) -> RuntimeState + Send + Sync>,
}

impl RuntimeState {
    pub(super) fn downgrade(&self) -> WeakRuntimeState {
        let config = self.config.clone();
        let transport = self.transport.clone();
        let development = self.development.clone();
        let activation_locks = self.activation_locks.clone();
        let publication_slots = self.publication_slots.clone();
        let frontend = self.frontend.clone();
        WeakRuntimeState {
            extensions: std::sync::Arc::downgrade(&self.extensions),
            restore: Box::new(move |extensions| RuntimeState {
                config: config.clone(),
                transport: transport.clone(),
                development: development.clone(),
                activation_locks: activation_locks.clone(),
                publication_slots: publication_slots.clone(),
                frontend: frontend.clone(),
                extensions,
            }),
        }
    }
}

impl WeakRuntimeState {
    pub(super) fn upgrade(&self) -> Result<RuntimeState> {
        let extensions = self.extensions.upgrade().context("运行时已关闭")?;
        Ok((self.restore)(extensions))
    }
}
