use super::super::delivery::DeliverySources;
use super::RuntimeAssembly;
use az_plugin_core::{Plugin, PluginFuture};

/// GitHub 发现器只依赖来源契约；可独立移除或使用替代队列，默认访问公共 GitHub。
/// api_base 由部署方指定，可使用企业 GitHub 的 /api/v3/，不开放给业务请求。
pub struct GithubDeliveryDiscoveryPlugin {
    pub api_base: String,
}
impl Default for GithubDeliveryDiscoveryPlugin {
    fn default() -> Self {
        Self {
            api_base: "https://api.github.com/".into(),
        }
    }
}
impl Plugin<RuntimeAssembly> for GithubDeliveryDiscoveryPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            let Some(config) = assembly.config.delivery.clone() else {
                return Ok(());
            };
            assembly.require_service::<dyn DeliverySources>();
            let discovery =
                super::super::delivery::discovery::GithubDiscovery::new(&self.api_base, config)?;
            assembly.services.add_value(discovery);
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move {
                    let discovery =
                        state.service::<super::super::delivery::discovery::GithubDiscovery>()?;
                    let sources = state.service::<dyn DeliverySources>()?;
                    tasks.spawn(discovery.run(sources, tasks.cancellation()))
                })
            });
            Ok(())
        })
    }
}
