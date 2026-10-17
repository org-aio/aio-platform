//! 部署方显式选择的首次整包引导，不依赖市场界面，也不替代租户的版本管理。

use std::{any::TypeId, path::PathBuf};

use az_plugin_core::{Plugin, PluginFuture};

use super::{ComponentsPlugin, RuntimeAssembly};
use crate::runtime::server::components::{Components, bootstrap::PreparedComponents};

/// 部署配置中的本地整包；摘要使用打包器输出的规范摘要，租户必须逐项声明。
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapComponent {
    pub path: PathBuf,
    pub digest: String,
    pub tenants: Vec<String>,
}

/// 可选的原生引导插件；应用组合层传入配置，首次安装仍走现有组件生命周期。
/// 更新配置后重新装配该插件；增加执行器会通过 Components 依赖自动重试未完成项。
pub struct ComponentBootstrapPlugin(pub Vec<BootstrapComponent>);

impl Plugin<RuntimeAssembly> for ComponentBootstrapPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<ComponentsPlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<Components>();
            let declarations = self.0.clone();
            // 在启动任何资源前检查整批文件；有界磁盘读取和解包不占用异步工作线程。
            let prepared =
                tokio::task::spawn_blocking(move || PreparedComponents::load(declarations))
                    .await??;
            assembly.services.add_value(prepared);
            assembly.on_start::<Self>(|state, _| {
                Box::pin(async move {
                    let prepared = state.service::<PreparedComponents>()?;
                    state.service::<Components>()?.bootstrap(&prepared).await
                })
            });
            Ok(())
        })
    }
}
