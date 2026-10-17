//! 基础设施由所选插件持有，异步初始化和失败补偿沿用宿主生命周期。
#[cfg(feature = "sidecar")]
use super::super::process::ProcessManager;
#[cfg(feature = "wasm")]
use super::super::wasm::WasmManager;
#[cfg(feature = "sidecar")]
use super::ProcessExecutor;
#[cfg(feature = "wasm")]
use super::WasmExecutor;

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use anyhow::{Context, Result};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_runtime::Keyring;
use sqlx::{PgPool, postgres::PgPoolOptions};

use super::super::{components::Components, repository::RepositoryInstaller, store::PluginStore};
use super::{ComponentExecutionPlugin, ComponentExecutors, Resource, RuntimeAssembly};
use crate::configuration::HostConfig;

/// 提供当前宿主拥有的数据库连接池；应用可替换连接建立方式，停止时统一关闭。
#[async_trait::async_trait]
pub trait DatabaseProvider: Any + Send + Sync {
    async fn connect(&self, config: &HostConfig) -> Result<PgPool>;
}

/// 默认 PostgreSQL 连接方式；独立于数据库资源的生命周期。
#[dill::component]
#[dill::interface(dyn DatabaseProvider)]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresPlugin;

#[async_trait::async_trait]
impl DatabaseProvider for PostgresPlugin {
    async fn connect(&self, config: &HostConfig) -> Result<PgPool> {
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&config.database_url)
            .await
            .context("连接插件运行时 PostgreSQL 失败")
    }
}

impl Plugin<RuntimeAssembly> for PostgresPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<PostgresPlugin>();
            Ok(())
        })
    }
}

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

/// 初始化数据库并持有关闭责任，具体连接实现通过 DatabaseProvider 注入。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct DatabasePlugin;

impl Plugin<RuntimeAssembly> for DatabasePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn DatabaseProvider>();
            assembly.declare_resource::<PgPool>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let pool = state
                        .service::<dyn DatabaseProvider>()?
                        .connect(&state.config)
                        .await?;
                    state.service::<Resource<PgPool>>()?.provide(Arc::new(pool))
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(pool) = state.service::<Resource<PgPool>>()?.initialized() {
                        pool.close().await;
                    }
                    Ok(())
                })
            });
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

/// 平台持久化结构及开发会话清理依赖数据库能力。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct StorePlugin;

impl Plugin<RuntimeAssembly> for StorePlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<DatabasePlugin>()]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<PluginStore>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let store = Arc::new(PluginStore::new(
                        state.service::<PgPool>()?.as_ref().clone(),
                    ));
                    store.migrate().await?;
                    if state.config.development.is_some() {
                        // 开发进程由本次 CLI 管理，旧地址不能在下次启动时复用。
                        sqlx::query(
                            "DELETE FROM tenant_plugin_bindings WHERE tenant_id='development'",
                        )
                        .execute(&store.pool)
                        .await?;
                    }
                    state.service::<Resource<PluginStore>>()?.provide(store)
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
            assembly
                .services
                .add_value(RepositoryInstaller::new(assembly.config.cache_root.clone()));
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
            TypeId::of::<StorePlugin>(),
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
            assembly.optional_service::<super::super::enhancements::Enhancements>();
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
                        state.service::<PgPool>()?.as_ref().clone(),
                        storage,
                        state.service::<Keyring>()?,
                        #[cfg(feature = "sidecar")]
                        state.service::<dyn crate::identity::IdentityProvider>()?,
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
