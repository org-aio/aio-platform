mod activation_store;
#[cfg(any(test, feature = "test-support"))]
mod admin_test_support;
mod bootstrap;
mod bootstrap_document;
mod capabilities;
mod components;
mod delivery;
pub mod development;
mod enhancements;
mod execution_availability;
mod frontend_access;
mod frontend_delivery;
mod frontend_devices;
mod frontend_document;
mod frontend_model;
mod frontend_package;
mod frontend_routes;
#[cfg(test)]
mod frontend_tests;
pub mod http_error;
mod installation;
mod lifecycle;
mod management;
mod management_dispatch;
mod marketplace_removal;
mod marketplace_store;
mod navigation;
mod official_market;
mod package_repository;
mod package_store;
mod page_state;
pub mod plugins;
#[cfg(feature = "sidecar")]
mod process;
mod protocols;
mod publication;
mod publication_validation;
mod publisher_store;
mod releases;
#[cfg(any(feature = "remote-mcp", feature = "remote-grpc"))]
mod remote;
mod remote_access;
mod repository;
pub(crate) mod request_context;
mod routes;
mod service_dispatch;
mod source_migration;
mod store;
#[cfg(feature = "sidecar")]
mod supervisor;
#[cfg(feature = "test-support")]
pub mod test_support;
mod tools;
pub(crate) mod transport;
#[cfg(feature = "wasm")]
mod wasm;
mod weak_state;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use anyhow::{Context as _, Result};

use crate::runtime::PublishState;

pub use bootstrap_document::document as bootstrap_document;
pub use routes::router;
#[cfg(feature = "sidecar")]
pub use supervisor::run as run_supervisor;

#[derive(Clone)]
pub struct RuntimeState {
    pub config: Arc<crate::configuration::HostConfig>,
    pub(crate) transport: transport::TransportOrigins,
    development: Arc<development::DevelopmentState>,
    extensions: Arc<plugins::RuntimeExtensions>,
    activation_locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
    publication_slots: Arc<tokio::sync::Semaphore>,
    frontend: Arc<frontend_access::FrontendAccess>,
}

impl RuntimeState {
    fn with_extensions(&self, extensions: Arc<plugins::RuntimeExtensions>) -> Self {
        Self {
            extensions,
            ..self.clone()
        }
    }

    /// 按接口类型取得已安装插件贡献的服务；未安装的能力不会隐式创建默认实现。
    pub fn service<I: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<I>> {
        self.extensions
            .services
            .get_one::<I>()
            .context("宿主未安装所需服务插件")
    }

    /// 当前代的唯一 Dill 容器，供既有 Controller 按类型装配路由。
    /// 保存容器的插件必须声明 observe_catalog；应用路由只在当前代使用该引用。
    pub fn catalog(&self) -> &dill::Catalog {
        &self.extensions.services
    }

    pub(crate) fn worker_keyring(&self) -> Result<az_plugin_runtime::Keyring> {
        Ok(self
            .service::<az_plugin_runtime::Keyring>()?
            .as_ref()
            .clone())
    }

    pub(crate) fn store(&self) -> Result<Arc<store::PluginStore>> {
        self.service()
    }
    pub(crate) fn repository(&self) -> Result<Arc<repository::RepositoryInstaller>> {
        self.service()
    }
    pub(crate) fn process(&self) -> Result<Arc<dyn plugins::ProcessExecutor>> {
        self.service()
    }
    pub(crate) fn wasm(&self) -> Result<Arc<dyn plugins::WasmExecutor>> {
        self.service()
    }

    /// 解析可选服务；缺失返回 None，重复绑定和构造失败仍返回错误。
    pub fn optional_service<I: ?Sized + Send + Sync + 'static>(&self) -> Result<Option<Arc<I>>> {
        if self.extensions.services.builders_for::<I>().count() == 0 {
            return Ok(None);
        }
        self.service().map(Some)
    }

    /// 由应用选择宿主能力组合，使用同一套依赖校验、服务绑定与启动流程。
    pub async fn initialize(
        config: crate::configuration::HostConfig,
        plugins: Vec<az_plugin_core::DynPlugin<plugins::RuntimeAssembly>>,
    ) -> Result<Self> {
        let run_hooks = config.development.is_none();
        Self::initialize_selected(config, plugins, run_hooks).await
    }

    async fn initialize_selected(
        config: crate::configuration::HostConfig,
        plugins: Vec<az_plugin_core::DynPlugin<plugins::RuntimeAssembly>>,
        run_hooks: bool,
    ) -> Result<Self> {
        let state = Self::assemble(config, plugins).await?;
        state.extensions.start(&state, run_hooks).await?;
        Ok(state)
    }

    async fn assemble(
        config: crate::configuration::HostConfig,
        plugins: Vec<az_plugin_core::DynPlugin<plugins::RuntimeAssembly>>,
    ) -> Result<Self> {
        if config.development.is_some() {
            anyhow::ensure!(
                config.delivery.is_none() && config.default_plugins.is_empty(),
                "开发宿主禁止启用生产交付与默认插件组合"
            );
        }
        let transport = transport::TransportOrigins::from_env(&config.public_origin)?;
        let config = Arc::new(config);
        let extensions = plugins::assemble(config.clone(), plugins).await?;
        let state = Self {
            transport,
            extensions,
            development: Arc::default(),
            activation_locks: Arc::new(Mutex::new(HashMap::new())),
            publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            frontend: Arc::new(frontend_access::FrontendAccess::new(&config.public_origin)?),
            config,
        };
        Ok(state)
    }

    fn components(&self) -> Result<Arc<components::Components>> {
        self.service().context("宿主尚未配置 Component 持久存储")
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

    fn start_publish_job(&self, job: publisher_store::PublishJob) -> Result<()> {
        if job.state != PublishState::Queued {
            return Ok(());
        }
        let state = self.clone();
        let tasks = self.plugin_tasks::<plugins::ManagementPlugin>()?;
        let cancellation = tasks.cancellation();
        tasks.spawn(async move {
            let _permit = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(()),
                permit = state.publication_slots.clone().acquire_owned() => permit?,
            };
            if cancellation.is_cancelled() {
                return Ok(());
            }
            let claimed = match state.store()?.claim_publish_job(&job.id).await {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!("领取插件发布任务失败: {}", error);
                    return Err(error);
                }
            };
            if !claimed {
                return Ok(());
            }
            let _ = state
                .store()?
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
                    .repository()?
                    .validate_published(&job.git, &job.revision)
                    .await?;
                let publication = state
                    .repository()?
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
                .store()?
                .finish_publish_job(&job.id, publish_state, &detail, page_count)
                .await
            {
                return Err(error.context("记录插件发布任务结果失败"));
            }
            if let Err(error) = state
                .store()?
                .record_lifecycle_event(&job.tenant_id, &job.source_id, None, lifecycle, &detail)
                .await
            {
                eprintln!("记录插件发布生命周期失败: {}", error);
                return Err(error);
            }
            Ok(())
        })
    }

    async fn resume_published_jobs(&self) -> Result<()> {
        for job in self.store()?.resume_publish_jobs().await? {
            self.start_publish_job(job)?;
        }
        Ok(())
    }

    async fn ensure_default_plugins(&self) -> Result<()> {
        if self.store()?.has_plugins("default").await? {
            return Ok(());
        }
        let mut plugins = Vec::with_capacity(self.config.default_plugins.len());
        for source in &self.config.default_plugins {
            plugins.push(
                self.repository()?
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
                .store()?
                .activate("default", plugin, process.as_ref(), None)
                .await
            {
                if let Some(process) = &process
                    && process.created
                {
                    let _ = self.process()?.stop(&process.instance_id).await;
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
        let Some(manager) = self.optional_service::<dyn plugins::ProcessExecutor>()? else {
            return Ok(());
        };
        self.store()?.stop_orphan_process_records().await?;
        let targets = self.store()?.enabled_process_targets().await?;
        for target in &targets {
            self.restore_package_cache(&target.revision).await?;
        }
        if let Err(error) = manager.health().await {
            if !targets.is_empty() {
                return Err(error.context("已安装 process 插件，需要可用的监督器"));
            }
            eprintln!("未启用 process 插件，监督器暂不可用: {error:#}");
            return Ok(());
        }
        self.process()?
            .reconcile(
                targets
                    .iter()
                    .map(|target| plugins::StartProcessRequest {
                        tenant_id: target.tenant_id.clone(),
                        source_id: target.source_id.clone(),
                        revision: target.revision.clone(),
                    })
                    .collect(),
            )
            .await?;
        for target in targets {
            let instance = self
                .process()?
                .start(&target.tenant_id, &target.source_id, &target.revision)
                .await
                .with_context(|| {
                    format!(
                        "恢复 process 插件失败: tenant={} source={} revision={}",
                        target.tenant_id, target.source_id, target.revision
                    )
                })?;
            let validation = async {
                let pages = self.process()?.load_pages(&instance.endpoint).await?;
                self.repository()?
                    .validate_pages(&target.revision, &pages)?;
                self.store()?
                    .verify_revision_pages(&target.revision_id, &pages)
                    .await
            }
            .await;
            if let Err(error) = validation {
                let _ = self.process()?.stop(&instance.instance_id).await;
                return Err(error.context("恢复 process 插件页面失败"));
            }
            if instance.created
                && let Err(error) = self
                    .store()?
                    .recover_process_instance(&target, &instance)
                    .await
            {
                let _ = self.process()?.stop(&instance.instance_id).await;
                return Err(error.context("记录恢复的 process 插件实例失败"));
            }
        }
        Ok(())
    }

    async fn reconcile_wasm(&self) -> Result<()> {
        if self
            .optional_service::<dyn plugins::WasmExecutor>()?
            .is_none()
        {
            return Ok(());
        }
        for target in self.store()?.enabled_wasm_targets().await? {
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
                self.repository()?
                    .validate_pages(&target.revision, &activation.pages)?;
                self.store()?
                    .verify_revision_pages(&target.revision_id, &activation.pages)
                    .await
            }
            .await;
            if let Err(error) = validation {
                self.wasm()?
                    .deactivate(&target.tenant_id, &target.source_id, &target.revision)?;
                return Err(error.context("恢复 Wasm Component 页面失败"));
            }
            if activation.created {
                self.store()?.recover_wasm_instance(&target).await?;
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
    ) -> Result<plugins::WasmActivation> {
        self.restore_package_cache(revision).await?;
        let artifact = self.repository()?.artifact(revision, artifact)?;
        let manager = self.wasm()?;
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
