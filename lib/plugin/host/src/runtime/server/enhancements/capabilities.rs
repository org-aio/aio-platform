use std::sync::{Arc, Weak};

use anyhow::{Context, Result};
use az_plugin_contract::InvocationScope;
use az_plugin_enhancement::{CapabilityCaller, InvocationCapabilities, InvocationControl};
use serde_json::Value;

use super::super::capabilities::{CapabilityInvocation, CapabilityLifetime, RuntimeCapabilities};
use crate::identity::IdentityProvider;

pub(in crate::runtime::server) struct CapabilityEnvironment {
    pub services: Weak<super::super::plugins::RuntimeExtensions>,
    pub identity: Arc<dyn IdentityProvider>,
}

impl CapabilityEnvironment {
    pub(super) fn runtime(&self) -> Result<Arc<RuntimeCapabilities>> {
        let services = self.services.upgrade().context("宿主能力环境已释放")?;
        Ok(services.services.get_one::<RuntimeCapabilities>()?)
    }

    pub(super) fn bind(&self, scope: &InvocationScope) -> Result<InvocationCapabilities> {
        let services = self.services.upgrade().context("宿主能力环境已释放")?;
        let invocation = self.runtime()?.bind(
            scope,
            self.identity.clone(),
            services.services.get_one::<CapabilityLifetime>()?.0.clone(),
            None,
        )?;
        Ok(InvocationCapabilities {
            grants: scope.grants.runtime.clone(),
            caller: Some(Arc::new(Caller {
                invocation,
                handle: tokio::runtime::Handle::try_current().context("宿主能力缺少异步运行时")?,
            })),
        })
    }
}

struct Caller {
    invocation: CapabilityInvocation,
    handle: tokio::runtime::Handle,
}

impl CapabilityCaller for Caller {
    fn available(&self, path: &str) -> bool {
        self.invocation.available(path)
    }

    fn call(&self, path: &str, input: &Value, control: &InvocationControl) -> Result<Value> {
        // 解释器只在 spawn_blocking 线程等待；与原生入口共用同一个异步执行链。
        self.handle.block_on(
            self.invocation
                .execute(path, input, control.deadline(), || control.check()),
        )
    }
}
