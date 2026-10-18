//! 基础设施由所选插件持有，异步初始化和失败补偿沿用宿主生命周期。
#[cfg(feature = "sidecar")]
use super::super::process::ProcessManager;
#[cfg(feature = "wasm")]
use super::super::wasm::WasmManager;
#[cfg(feature = "sidecar")]
use super::ProcessExecutor;
use super::WasmExecutor;

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use anyhow::{Context, Result};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_runtime::Keyring;

use super::super::{components::Components, repository::RepositoryInstaller};
use super::{ComponentExecutionPlugin, ComponentExecutors, Resource, RuntimeAssembly};
use crate::configuration::HostConfig;

/// 密钥来源可由文件、外部密钥管理服务或应用自己的实现提供。
#[async_trait::async_trait]
pub trait KeyringProvider: Any + Send + Sync {
    async fn load(&self, config: &HostConfig) -> Result<Keyring>;
}

/// 默认文件密钥来源；只有安装对应能力才会读取或创建文件。
#[dill::component]
#[dill::interface(dyn KeyringProvider)]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct FileKeyringPlugin;

#[async_trait::async_trait]
impl KeyringProvider for FileKeyringPlugin {
    async fn load(&self, config: &HostConfig) -> Result<Keyring> {
        let path = config
            .component_storage
            .as_ref()
            .map(|storage| storage.root.join("keyring.json"))
            .unwrap_or_else(|| config.cache_root.join("worker-keyring.json"));
        super::super::components::load_keyring(&path)
    }
}

impl Plugin<RuntimeAssembly> for FileKeyringPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<FileKeyringPlugin>();
            Ok(())
        })
    }
}

/// 加载共享密钥；所有调用通道复用同一个实例。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct KeyringPlugin;

impl Plugin<RuntimeAssembly> for KeyringPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn KeyringProvider>();
            assembly.declare_resource::<Keyring>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let keyring = state
                        .service::<dyn KeyringProvider>()?
                        .load(&state.config)
                        .await?;
                    state
                        .service::<Resource<Keyring>>()?
                        .provide(Arc::new(keyring))
                })
            });
            Ok(())
        })
    }
}

/// 发布包缓存由仓库插件提供，不要求启动数据库或执行引擎。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct RepositoryPlugin;

impl Plugin<RuntimeAssembly> for RepositoryPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.optional_service::<dyn WasmExecutor>();
            assembly.declare_resource::<RepositoryInstaller>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let repository = RepositoryInstaller::new(
                        state.config.cache_root.clone(),
                        state.optional_service::<dyn WasmExecutor>()?,
                    );
                    state
                        .service::<Resource<RepositoryInstaller>>()?
                        .provide(Arc::new(repository))
                })
            });
            Ok(())
        })
    }
}

/// 旧进程执行资源的初始化与停止，不承担路由注册。
#[cfg(feature = "sidecar")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ProcessPlugin;

#[cfg(feature = "sidecar")]
impl Plugin<RuntimeAssembly> for ProcessPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<ProcessManager>()?;
            assembly
                .services
                .bind::<dyn ProcessExecutor, ProcessManager>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let process = if state.config.development.is_some() {
                        ProcessManager::local()?
                    } else {
                        ProcessManager::new()?
                    };
                    state
                        .service::<Resource<ProcessManager>>()?
                        .provide(Arc::new(process))
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(process) =
                        state.service::<Resource<ProcessManager>>()?.initialized()
                    {
                        process.shutdown().await?;
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

/// 旧 Wasm 引擎的初始化与停止，不承担路由注册。
#[cfg(feature = "wasm")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct WasmPlugin;

#[cfg(feature = "wasm")]
impl Plugin<RuntimeAssembly> for WasmPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<WasmManager>()?;
            assembly.services.bind::<dyn WasmExecutor, WasmManager>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let wasm =
                        WasmManager::with_cache(Some(&state.config.cache_root.join("compiled")))?;
                    state
                        .service::<Resource<WasmManager>>()?
                        .provide(Arc::new(wasm))
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(wasm) = state.service::<Resource<WasmManager>>()?.initialized() {
                        wasm.shutdown()?;
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

/// 组件执行环境的资源所有者；未配置组件存储时不创建引擎与附加连接池。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ComponentResourcesPlugin;

impl Plugin<RuntimeAssembly> for ComponentResourcesPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<KeyringPlugin>(),
            TypeId::of::<ComponentExecutionPlugin>(),
        ]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            if assembly.config.component_storage.is_none() {
                return Ok(());
            }
            assembly.require_service::<dyn super::ComponentDatabaseProvider>();
            assembly.require_service::<dyn super::ObjectStorageProvider>();
            assembly.require_service::<dyn super::ComponentCatalog>();
            // 组件环境持有同一身份实例；不能依赖脚本服务间接提供初始化顺序。
            #[cfg(feature = "sidecar")]
            assembly.require_service::<dyn crate::identity::IdentityProvider>();
            assembly.optional_service::<super::super::enhancements::Enhancements>();
            assembly.optional_service::<dyn crate::identity::AccessControlProvider>();
            assembly.optional_service::<super::ManagementAccess>();
            assembly.observe_catalog();
            assembly.declare_resource::<Components>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let storage = state
                        .config
                        .component_storage
                        .as_ref()
                        .context("未配置组件存储")?;
                    let components = Components::open(
                        state.service::<dyn super::ComponentCatalog>()?,
                        storage,
                        state.service::<Keyring>()?,
                        super::super::components::Providers {
                            #[cfg(feature = "sidecar")]
                            identity: state.service::<dyn crate::identity::IdentityProvider>()?,
                            #[cfg(feature = "sidecar")]
                            access_control: state
                                .optional_service::<dyn crate::identity::AccessControlProvider>()?,
                        },
                        state.extensions.services.weak_ref(),
                        state.optional_service::<super::super::enhancements::Enhancements>()?,
                        state.service::<ComponentExecutors>()?,
                    )
                    .await?;
                    state.service::<Resource<Components>>()?.provide(components)
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(components) = state.service::<Resource<Components>>()?.initialized()
                    {
                        components.shutdown().await?;
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}
