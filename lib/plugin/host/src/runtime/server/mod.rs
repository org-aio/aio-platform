mod activation_store;
#[cfg(any(test, feature = "test-support"))]
mod admin_test_support;
mod bootstrap;
mod bootstrap_document;
mod components;
mod delivery;
pub mod development;
mod frontend_access;
mod frontend_delivery;
mod frontend_document;
mod frontend_model;
mod frontend_package;
mod frontend_routes;
#[cfg(test)]
mod frontend_tests;
pub(crate) mod http_error;
mod installation;
mod lifecycle;
mod management;
mod marketplace_store;
mod navigation;
mod official_market;
mod package_repository;
mod package_store;
mod page_state;
mod process;
mod publication;
mod publication_validation;
mod publisher_store;
mod releases;
mod remote_access;
mod repository;
pub(crate) mod request_context;
mod routes;
mod service_dispatch;
mod source_migration;
mod store;
mod supervisor;
#[cfg(feature = "test-support")]
pub mod test_support;
mod tools;
mod wasm;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use anyhow::{Context as _, Result};
use sqlx::postgres::PgPoolOptions;

use crate::runtime::PublishState;

pub use bootstrap_document::document as bootstrap_document;
pub use routes::router;
pub use supervisor::run as run_supervisor;

#[derive(Clone)]
pub struct RuntimeState {
    pub config: Arc<crate::configuration::HostConfig>,
    development: Arc<development::DevelopmentState>,
    pub store: Arc<store::PluginStore>,
    pub repository: Arc<repository::RepositoryInstaller>,
    pub identity: Arc<dyn crate::identity::IdentityProvider>,
    pub(crate) workers: Arc<dyn crate::generated::worker::WorkerService>,
    activation_locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
    publication_slots: Arc<tokio::sync::Semaphore>,
    frontend: Arc<frontend_access::FrontendAccess>,
    pub process: Arc<process::ProcessManager>,
    pub wasm: Arc<wasm::WasmManager>,
    components: Option<Arc<components::Components>>,
}

impl RuntimeState {
    pub(crate) fn worker_keyring(&self) -> Result<az_plugin_runtime::Keyring> {
        let path = self
            .config
            .component_storage
            .as_ref()
            .map(|v| v.root.join("keyring.json"))
            .unwrap_or_else(|| self.config.cache_root.join("worker-keyring.json"));
        components::load_keyring(&path)
    }

    pub async fn initialize(
        config: crate::configuration::HostConfig,
        identity: Arc<dyn crate::identity::IdentityProvider>,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(&config.database_url)
            .await
            .context("连接插件运行时 PostgreSQL 失败")?;
        let store = Arc::new(store::PluginStore::new(pool));
        store.migrate().await?;
        let cache_root = config.cache_root.clone();
        let repository = Arc::new(repository::RepositoryInstaller::new(cache_root.clone()));
        let process = Arc::new(if config.development.is_some() {
            anyhow::ensure!(
                config.delivery.is_none() && config.default_plugins.is_empty(),
                "开发宿主禁止启用生产交付与默认插件组合"
            );
            process::ProcessManager::local()?
        } else {
            process::ProcessManager::new()?
        });
        let wasm = Arc::new(wasm::WasmManager::with_cache(Some(
            &cache_root.join("compiled"),
        ))?);
        let components = if let Some(storage) = &config.component_storage {
            let root = &storage.root;
            Some(
                components::Components::open(
                    store.pool.clone(),
                    &storage.database_url,
                    &root.join("keyring.json"),
                    root.join("objects"),
                    identity.clone(),
                )
                .await?,
            )
        } else {
            None
        };
        let workers = dill::Catalog::builder()
            .add_value(store.pool.clone())
            .add::<crate::generated::worker::WorkerServiceImpl>()
            .build()
            .get_one::<dyn crate::generated::worker::WorkerService>()?;
        let state = Self {
            workers,
            development: Arc::default(),
            store,
            repository,
            identity,
            process,
            wasm,
            components,
            activation_locks: Arc::new(Mutex::new(HashMap::new())),
            publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            frontend: Arc::new(frontend_access::FrontendAccess::new(&config.public_origin)?),
            config: Arc::new(config),
        };
        if state.config.development.is_some() {
            // 开发进程由本次 CLI 管理，旧进程地址不能在下次启动时复用。
            sqlx::query("DELETE FROM tenant_plugin_bindings WHERE tenant_id='development'")
                .execute(&state.store.pool)
                .await?;
            return Ok(state);
        }
        state.ensure_default_plugins().await?;
        state.reconcile_wasm().await?;
        state.reconcile_processes().await?;
        if let Some(components) = &state.components {
            components.restore().await?;
        }
        state.resume_published_jobs().await?;
        delivery::start(state.clone());
        Ok(state)
    }

    fn components(&self) -> Result<&components::Components> {
        self.components
            .as_deref()
            .context("宿主尚未配置 Component 持久存储")
    }

    pub(super) fn activation_lock(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Arc<tokio::sync::Mutex<()>>> {
        let key = format!("{tenant_id}\0{source_id}");
        let mut locks = self
            .activation_locks
            .lock()
            .map_err(|_| anyhow::anyhow!("插件生命周期锁已损坏"))?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        Ok(lock)
    }

    fn start_publish_job(&self, job: publisher_store::PublishJob) {
        if job.state != PublishState::Queued {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            let Ok(_permit) = state.publication_slots.clone().acquire_owned().await else {
                return;
            };
            let claimed = match state.store.claim_publish_job(&job.id).await {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!("领取插件发布任务失败: {}", error);
                    return;
                }
            };
            if !claimed {
                return;
            }
            let _ = state
                .store
                .record_lifecycle_event(
                    &job.tenant_id,
                    &job.source_id,
                    None,
                    "publish-verify",
                    "正在校验运行时 artifact、页面定义和健康检查",
                )
                .await;
            let result = async {
                state.restore_package_cache(&job.revision).await?;
                let discovered = state
                    .repository
                    .validate_published(&job.git, &job.revision)
                    .await?;
                let publication = state
                    .repository
                    .published_marketplace_entry(&job.git, &job.revision)?;
                let activated = installation::activate(
                    &state,
                    &job.tenant_id,
                    discovered,
                    "已校验二进制插件包内容摘要、能力声明和运行时协议",
                    Some(&publication),
                )
                .await?;
                Ok::<_, anyhow::Error>(activated)
            }
            .await;
            let (publish_state, lifecycle, detail, page_count) = match result {
                Ok(activated) => {
                    let detail = format!(
                        "版本 {} 已在线激活，共 {} 个页面",
                        activated.revision, activated.page_count
                    );
                    (
                        PublishState::Active,
                        "publish-active",
                        detail,
                        Some(activated.page_count),
                    )
                }
                Err(error) => (
                    PublishState::Failed,
                    "publish-failed",
                    format!("后台验证或激活失败，已保留上一活动版本: {error:#}"),
                    None,
                ),
            };
            if let Err(error) = state
                .store
                .finish_publish_job(&job.id, publish_state, &detail, page_count)
                .await
            {
                eprintln!("记录插件发布任务结果失败: {}", error);
            }
            if let Err(error) = state
                .store
                .record_lifecycle_event(&job.tenant_id, &job.source_id, None, lifecycle, &detail)
                .await
            {
                eprintln!("记录插件发布生命周期失败: {}", error);
            }
        });
    }

    async fn resume_published_jobs(&self) -> Result<()> {
        for job in self.store.resume_publish_jobs().await? {
            self.start_publish_job(job);
        }
        Ok(())
    }

    async fn ensure_default_plugins(&self) -> Result<()> {
        if self.store.has_plugins("default").await? {
            return Ok(());
        }
        let mut plugins = Vec::with_capacity(self.config.default_plugins.len());
        for source in &self.config.default_plugins {
            plugins.push(
                self.repository
                    .discover(&source.git, Some(&source.rev))
                    .await?,
            );
        }
        for mut plugin in plugins {
            let process = if plugin.runtime == crate::runtime::PluginRuntime::Process {
                Some(lifecycle::prepare_process(self, "default", &mut plugin).await?)
            } else {
                None
            };
            let wasm = if plugin.runtime == crate::runtime::PluginRuntime::WasmComponent {
                Some(lifecycle::prepare_wasm(self, "default", &mut plugin).await?)
            } else {
                None
            };
            let source_id = plugin.source_id.clone();
            let revision = plugin.revision.clone();
            if let Err(error) = self
                .store
                .activate("default", plugin, process.as_ref(), None)
                .await
            {
                if let Some(process) = &process
                    && process.created
                {
                    let _ = self.process.stop(&process.instance_id).await;
                }
                let _ = lifecycle::cleanup_new_wasm(
                    self,
                    "default",
                    &source_id,
                    &revision,
                    wasm.as_ref(),
                );
                return Err(error.context("激活默认插件组合失败"));
            }
        }
        Ok(())
    }

    async fn reconcile_processes(&self) -> Result<()> {
        self.store.stop_orphan_process_records().await?;
        let targets = self.store.enabled_process_targets().await?;
        for target in &targets {
            self.restore_package_cache(&target.revision).await?;
        }
        if let Err(error) = self.process.health().await {
            if !targets.is_empty() {
                return Err(error.context("已安装 process 插件，需要可用的监督器"));
            }
            eprintln!("未启用 process 插件，监督器暂不可用: {error:#}");
            return Ok(());
        }
        self.process
            .reconcile(
                targets
                    .iter()
                    .map(|target| supervisor::StartProcessRequest {
                        tenant_id: target.tenant_id.clone(),
                        source_id: target.source_id.clone(),
                        revision: target.revision.clone(),
                    })
                    .collect(),
            )
            .await?;
        for target in targets {
            let instance = self
                .process
                .start(&target.tenant_id, &target.source_id, &target.revision)
                .await
                .with_context(|| {
                    format!(
                        "恢复 process 插件失败: tenant={} source={} revision={}",
                        target.tenant_id, target.source_id, target.revision
                    )
                })?;
            let validation = async {
                let pages = self.process.load_pages(&instance.endpoint).await?;
                self.repository.validate_pages(&target.revision, &pages)?;
                self.store
                    .verify_revision_pages(&target.revision_id, &pages)
                    .await
            }
            .await;
            if let Err(error) = validation {
                let _ = self.process.stop(&instance.instance_id).await;
                return Err(error.context("恢复 process 插件页面失败"));
            }
            if instance.created
                && let Err(error) = self
                    .store
                    .recover_process_instance(&target, &instance)
                    .await
            {
                let _ = self.process.stop(&instance.instance_id).await;
                return Err(error.context("记录恢复的 process 插件实例失败"));
            }
        }
        Ok(())
    }

    async fn reconcile_wasm(&self) -> Result<()> {
        for target in self.store.enabled_wasm_targets().await? {
            let activation = self
                .activate_wasm(
                    &target.tenant_id,
                    &target.source_id,
                    &target.revision,
                    &target.artifact,
                )
                .await
                .with_context(|| {
                    format!(
                        "恢复 Wasm Component 失败: tenant={} source={} revision={}",
                        target.tenant_id, target.source_id, target.revision
                    )
                })?;
            let validation = async {
                self.repository
                    .validate_pages(&target.revision, &activation.pages)?;
                self.store
                    .verify_revision_pages(&target.revision_id, &activation.pages)
                    .await
            }
            .await;
            if let Err(error) = validation {
                self.wasm
                    .deactivate(&target.tenant_id, &target.source_id, &target.revision)?;
                return Err(error.context("恢复 Wasm Component 页面失败"));
            }
            if activation.created {
                self.store.recover_wasm_instance(&target).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn activate_wasm(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        artifact: &str,
    ) -> Result<wasm::WasmActivation> {
        self.restore_package_cache(revision).await?;
        let artifact = self.repository.artifact(revision, artifact)?;
        let manager = self.wasm.clone();
        let tenant_id = tenant_id.to_owned();
        let source_id = source_id.to_owned();
        let revision = revision.to_owned();
        tokio::task::spawn_blocking(move || {
            manager.activate(&tenant_id, &source_id, &revision, &artifact)
        })
        .await
        .context("等待 Wasm Component 实例化失败")?
    }
}
