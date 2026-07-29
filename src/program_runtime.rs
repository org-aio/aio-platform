use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{
    ApplicationImage, CompileFailure, ComponentCatalog, GraphVm, GraphVmHost, ImageTarget,
    ProgramCompiler, ProgramDefinition, RevisionRunSnapshot, SegmentInvocationRequest,
    SegmentInvocationResult, SymbolId, VmEffect,
};
use anyhow::{Context, Result, bail};
use arc_swap::ArcSwapOption;
use matchit::Router;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::postgres::PgListener;
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::{capability::CapabilityCatalog, program_store::ProgramStore};
use az_plugin_core::{PageParams, RecordStore};

pub const PROGRAM_COMPILER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), ":program-v4");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramActivationEvent {
    pub application_id: String,
    pub revision_id: String,
    pub content_hash: String,
}

/// 运行时 Image 同时持有可序列化产物和预构建路由。
pub struct RuntimeApplicationImage {
    image: ApplicationImage,
    router: Router<SymbolId>,
}

impl RuntimeApplicationImage {
    pub fn build(image: ApplicationImage) -> Result<Self> {
        let mut router = Router::new();
        for route in &image.routes {
            router
                .insert(&route.path, route.id)
                .with_context(|| format!("预构建路由失败: {}", route.path))?;
        }
        Ok(Self { image, router })
    }

    #[must_use]
    pub fn image(&self) -> &ApplicationImage {
        &self.image
    }

    pub fn route(&self, path: &str) -> Result<RuntimeRouteMatch> {
        let matched = self
            .router
            .at(path)
            .with_context(|| format!("program route not found: {path}"))?;
        let route = self
            .image
            .routes
            .iter()
            .find(|route| route.id == *matched.value)
            .with_context(|| format!("compiled route metadata missing: {}", matched.value))?;
        Ok(RuntimeRouteMatch {
            route_id: route.id,
            page_id: route.page_id,
            parameters: matched
                .params
                .iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRouteMatch {
    pub route_id: SymbolId,
    pub page_id: SymbolId,
    pub parameters: Vec<(String, String)>,
}

type ImageSlot = Arc<ArcSwapOption<RuntimeApplicationImage>>;

/// 数据库程序的编译、发布、热切换与多实例同步入口。
#[derive(Clone)]
pub struct ProgramRuntime {
    store: ProgramStore,
    data_store: RecordStore,
    components: Arc<ComponentCatalog>,
    capability_catalog: Arc<CapabilityCatalog>,
    capabilities: Arc<crate::CapabilityCatalog>,
    slots: Arc<RwLock<HashMap<String, ImageSlot>>>,
    pending_generations: Arc<Mutex<HashMap<String, u64>>>,
    events: broadcast::Sender<ProgramActivationEvent>,
}

impl ProgramRuntime {
    #[must_use]
    pub fn new(
        store: ProgramStore,
        data_store: RecordStore,
        components: ComponentCatalog,
        capability_catalog: CapabilityCatalog,
    ) -> Self {
        let (events, _) = broadcast::channel(128);
        let capabilities = capability_catalog.program_catalog();
        Self {
            store,
            data_store,
            components: Arc::new(components),
            capability_catalog: Arc::new(capability_catalog),
            capabilities: Arc::new(capabilities),
            slots: Arc::new(RwLock::new(HashMap::new())),
            pending_generations: Arc::new(Mutex::new(HashMap::new())),
            events,
        }
    }

    #[must_use]
    pub fn store(&self) -> &ProgramStore {
        &self.store
    }

    #[must_use]
    pub fn component_catalog(&self) -> &ComponentCatalog {
        &self.components
    }

    #[must_use]
    pub fn capability_catalog(&self) -> &crate::CapabilityCatalog {
        &self.capabilities
    }

    pub fn validate_definition(
        &self,
        definition: &ProgramDefinition,
    ) -> Result<ApplicationImage, CompileFailure> {
        ProgramCompiler::new(
            PROGRAM_COMPILER_VERSION,
            &self.components,
            &self.capabilities,
        )
        .compile(definition, "validation", ImageTarget::Universal)
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ProgramActivationEvent> {
        self.events.subscribe()
    }

    /// 请求入口取得 Arc 后可跨越发布过程安全使用旧 Image。
    pub async fn image(&self, application_id: &str) -> Option<Arc<RuntimeApplicationImage>> {
        let slot = {
            let slots = self.slots.read().await;
            slots.get(application_id).cloned()
        }?;
        slot.load_full()
    }

    pub async fn active_images(&self) -> Vec<(String, Arc<RuntimeApplicationImage>)> {
        let values = {
            let slots = self.slots.read().await;
            slots
                .iter()
                .map(|(application_id, slot)| (application_id.clone(), slot.clone()))
                .collect::<Vec<_>>()
        };
        values
            .into_iter()
            .filter_map(|(application_id, slot)| {
                slot.load_full().map(|image| (application_id, image))
            })
            .collect()
    }

    pub async fn resolve_route(
        &self,
        application_id: &str,
        path: &str,
    ) -> Result<(Arc<RuntimeApplicationImage>, RuntimeRouteMatch)> {
        let image = self
            .image(application_id)
            .await
            .with_context(|| format!("active application image not found: {application_id}"))?;
        let route = image.route(path)?;
        Ok((image, route))
    }

    pub async fn invoke_server_segment(
        &self,
        application_id: &str,
        function_id: SymbolId,
        request: &SegmentInvocationRequest,
    ) -> Result<SegmentInvocationResult> {
        let image = self
            .image(application_id)
            .await
            .with_context(|| format!("active application image not found: {application_id}"))?;
        if !image.image().server_functions.contains_key(&function_id) {
            bail!("published server segment not found: {function_id}");
        }
        let mut host = ServerVmHost {
            data_store: self.data_store.clone(),
            capabilities: Arc::clone(&self.capability_catalog),
            image: image.image(),
        };
        let value = GraphVm::new(&image.image().server_functions)
            .execute(function_id, &request.inputs, &mut host)
            .await?;
        Ok(SegmentInvocationResult { value })
    }

    pub async fn restore_active_images(&self) -> Result<()> {
        for application in self.store.active_applications().await? {
            let Some(revision_id) = application.active_revision_id else {
                continue;
            };
            self.load_revision(&application.id, &revision_id, false)
                .await
                .with_context(|| format!("恢复活动 Image 失败: {}", application.id))?;
        }
        Ok(())
    }

    pub async fn publish_unactivated_applications(&self) -> Result<()> {
        for application in self.store.unactivated_applications().await? {
            self.publish_latest(&application.id, "migration")
                .await
                .with_context(|| format!("首次发布 Application 失败: {}", application.id))?;
        }
        Ok(())
    }

    /// 同一应用连续变更只保留 300ms 窗口内的最后一版。
    pub async fn schedule_publish(&self, application_id: String, origin: String) {
        let generation = {
            let mut pending = self.pending_generations.lock().await;
            let value = pending.entry(application_id.clone()).or_default();
            *value = value.saturating_add(1);
            *value
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let is_latest = {
                let pending = runtime.pending_generations.lock().await;
                pending.get(&application_id).copied() == Some(generation)
            };
            if !is_latest {
                return;
            }
            if let Err(error) = runtime.publish_latest(&application_id, &origin).await {
                tracing::error!(
                    application_id,
                    error = %format!("{error:#}"),
                    "自动发布 ProgramGraph 失败",
                );
            }
        });
    }

    pub async fn publish_latest(
        &self,
        application_id: &str,
        origin: &str,
    ) -> Result<Arc<RuntimeApplicationImage>> {
        let run = self.store.start_revision_run(application_id).await?;
        let draft = self.store.draft(application_id).await?;
        let compiler = ProgramCompiler::new(
            PROGRAM_COMPILER_VERSION,
            &self.components,
            &self.capabilities,
        );
        let mut image = match compiler.compile(&draft.definition, "pending", ImageTarget::Universal)
        {
            Ok(image) => image,
            Err(failure) => {
                let diagnostics =
                    serde_json::to_value(&failure.diagnostics).context("序列化编译诊断失败")?;
                let revision = self
                    .store
                    .create_revision(application_id, draft.definition, origin, &diagnostics)
                    .await?;
                self.finish_failed_run(&run, Some(&revision.id), "compile", &diagnostics)
                    .await?;
                return Err(failure.into());
            }
        };
        let diagnostics = Value::Array(Vec::new());
        let revision = self
            .store
            .create_revision(application_id, draft.definition, origin, &diagnostics)
            .await?;
        image.revision_id = revision.id.clone();

        let cached = self
            .store
            .image(
                &image.content_hash,
                PROGRAM_COMPILER_VERSION,
                ImageTarget::Universal,
            )
            .await?;
        let mut image = cached.unwrap_or(image);
        image.revision_id = revision.id.clone();
        image.application_id = revision.definition.id;
        let warmed = match RuntimeApplicationImage::build(image.clone()) {
            Ok(value) => Arc::new(value),
            Err(error) => {
                let diagnostics = json!([{
                    "code": "PROGRAM_PREWARM_FAILED",
                    "severity": "error",
                    "message": error.to_string(),
                    "stage": "smoke_test"
                }]);
                self.finish_failed_run(&run, Some(&revision.id), "prewarm", &diagnostics)
                    .await?;
                return Err(error.context("ApplicationImage 预热失败，活动版本保持不变"));
            }
        };

        if let Err(error) = self
            .store
            .reconcile_program_models(application_id, &revision.definition, &image)
            .await
        {
            let diagnostics = json!([{
                "code": "PROGRAM_MODEL_RECONCILE_FAILED",
                "severity": "error",
                "message": error.to_string(),
                "stage": "prewarm"
            }]);
            self.finish_failed_run(&run, Some(&revision.id), "prewarm", &diagnostics)
                .await?;
            return Err(error.context("动态模型或表达式索引预热失败，活动版本保持不变"));
        }

        self.store.save_image(&image).await?;
        self.store
            .activate_revision(application_id, &revision.id)
            .await?;
        self.swap_image(application_id, Arc::clone(&warmed)).await;
        let event = ProgramActivationEvent {
            application_id: application_id.to_owned(),
            revision_id: revision.id.clone(),
            content_hash: image.content_hash.clone(),
        };
        let subscribers = self.events.receiver_count();
        let delivered = self.events.send(event).unwrap_or_default();
        tracing::debug!(subscribers, delivered, "广播 ProgramGraph 激活事件");
        self.store
            .finish_revision_run(
                &run,
                Some(&revision.id),
                true,
                "activated",
                &diagnostics,
                &json!([{"name": "smoke_test", "status": "succeeded"}]),
            )
            .await?;
        Ok(warmed)
    }

    pub async fn load_revision(
        &self,
        application_id: &str,
        revision_id: &str,
        emit_event: bool,
    ) -> Result<Arc<RuntimeApplicationImage>> {
        let application = self.store.application(application_id).await?;
        if application.active_revision_id.as_deref() != Some(revision_id) {
            bail!("revision is no longer active: {revision_id}");
        }
        let revision = self.store.revision(revision_id).await?;
        let content_hash =
            crate::content_hash(&revision.definition).context("校验 Revision 内容哈希失败")?;
        let mut image = match self
            .store
            .image(
                &content_hash,
                PROGRAM_COMPILER_VERSION,
                ImageTarget::Universal,
            )
            .await?
        {
            Some(image) => image,
            None => ProgramCompiler::new(
                PROGRAM_COMPILER_VERSION,
                &self.components,
                &self.capabilities,
            )
            .compile(&revision.definition, revision_id, ImageTarget::Universal)
            .map_err(anyhow::Error::from)?,
        };
        image.revision_id = revision_id.to_owned();
        image.application_id = revision.definition.id;
        self.store.save_image(&image).await?;
        let content_hash = image.content_hash.clone();
        let warmed = Arc::new(RuntimeApplicationImage::build(image)?);
        self.swap_image(application_id, Arc::clone(&warmed)).await;
        if emit_event {
            let _ = self.events.send(ProgramActivationEvent {
                application_id: application_id.to_owned(),
                revision_id: revision_id.to_owned(),
                content_hash,
            });
        }
        Ok(warmed)
    }

    pub async fn activate_existing_revision(
        &self,
        application_id: &str,
        revision_id: &str,
    ) -> Result<Arc<RuntimeApplicationImage>> {
        let revision = self.store.revision(revision_id).await?;
        if revision.application_id != application_id {
            bail!("revision not found in application: {revision_id}");
        }
        let content_hash =
            crate::content_hash(&revision.definition).context("校验 Revision 内容哈希失败")?;
        let mut image = match self
            .store
            .image(
                &content_hash,
                PROGRAM_COMPILER_VERSION,
                ImageTarget::Universal,
            )
            .await?
        {
            Some(image) => image,
            None => ProgramCompiler::new(
                PROGRAM_COMPILER_VERSION,
                &self.components,
                &self.capabilities,
            )
            .compile(&revision.definition, revision_id, ImageTarget::Universal)
            .map_err(anyhow::Error::from)?,
        };
        image.revision_id = revision_id.to_owned();
        image.application_id = revision.definition.id;
        self.store.save_image(&image).await?;
        let content_hash = image.content_hash.clone();
        let warmed = Arc::new(RuntimeApplicationImage::build(image)?);
        self.store
            .activate_revision(application_id, revision_id)
            .await?;
        self.swap_image(application_id, Arc::clone(&warmed)).await;
        let _ = self.events.send(ProgramActivationEvent {
            application_id: application_id.to_owned(),
            revision_id: revision_id.to_owned(),
            content_hash,
        });
        Ok(warmed)
    }

    /// 每个实例持有独立 LISTEN 连接，收到通知后从不可变 revision/cache 恢复。
    pub async fn spawn_postgres_listener(&self, database_url: &str) -> Result<()> {
        let mut listener = PgListener::connect(database_url)
            .await
            .context("连接 ProgramGraph LISTEN 通道失败")?;
        listener
            .listen("engine_program_activated")
            .await
            .context("订阅 ProgramGraph 激活通知失败")?;
        let runtime = self.clone();
        tokio::spawn(async move {
            loop {
                let notification = match listener.recv().await {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(error = %error, "接收 ProgramGraph 激活通知失败");
                        break;
                    }
                };
                let payload =
                    match serde_json::from_str::<ActivationNotification>(notification.payload()) {
                        Ok(value) => value,
                        Err(error) => {
                            tracing::error!(error = %error, "解析 ProgramGraph 激活通知失败");
                            continue;
                        }
                    };
                let already_loaded = runtime
                    .image(&payload.application_id)
                    .await
                    .is_some_and(|image| image.image().revision_id == payload.revision_id);
                if already_loaded {
                    continue;
                }
                if let Err(error) = runtime
                    .load_revision(&payload.application_id, &payload.revision_id, true)
                    .await
                {
                    tracing::error!(
                        application_id = payload.application_id,
                        revision_id = payload.revision_id,
                        error = %format!("{error:#}"),
                        "加载其他实例发布的 ProgramGraph 失败",
                    );
                }
            }
        });
        Ok(())
    }

    async fn swap_image(&self, application_id: &str, image: Arc<RuntimeApplicationImage>) {
        let slot = {
            let mut slots = self.slots.write().await;
            slots
                .entry(application_id.to_owned())
                .or_insert_with(|| Arc::new(ArcSwapOption::empty()))
                .clone()
        };
        slot.store(Some(image));
    }

    async fn finish_failed_run(
        &self,
        run: &RevisionRunSnapshot,
        revision_id: Option<&str>,
        stage: &str,
        diagnostics: &Value,
    ) -> Result<()> {
        self.store
            .finish_revision_run(
                run,
                revision_id,
                false,
                stage,
                diagnostics,
                &Value::Array(Vec::new()),
            )
            .await
    }
}

#[derive(Deserialize)]
struct ActivationNotification {
    application_id: String,
    revision_id: String,
}

struct ServerVmHost<'a> {
    data_store: RecordStore,
    capabilities: Arc<CapabilityCatalog>,
    image: &'a ApplicationImage,
}

impl GraphVmHost for ServerVmHost<'_> {
    async fn apply(&mut self, effect: VmEffect) -> Result<Value> {
        match effect {
            VmEffect::CreateRecord { model_id, value } => {
                let model = self.model_name(model_id)?;
                serde_json::to_value(
                    self.data_store
                        .executor()
                        .insert_record(model, value)
                        .await?,
                )
                .context("序列化新增记录结果失败")
            }
            VmEffect::ReadRecord { model_id, value } => {
                let model = self.model_name(model_id)?;
                let record_id = record_id(&value)?;
                serde_json::to_value(
                    self.data_store
                        .executor()
                        .get_record(model, record_id)
                        .await?,
                )
                .context("序列化单条记录结果失败")
            }
            VmEffect::UpdateRecord { model_id, value } => {
                let model = self.model_name(model_id)?;
                let record_id = record_id(&value)?.to_owned();
                let payload = value.get("payload").cloned().unwrap_or(Value::Null);
                serde_json::to_value(
                    self.data_store
                        .executor()
                        .update_record(model, &record_id, payload)
                        .await?,
                )
                .context("序列化更新记录结果失败")
            }
            VmEffect::DeleteRecord { model_id, value } => {
                let model = self.model_name(model_id)?;
                let record_id = record_id(&value)?;
                self.data_store
                    .executor()
                    .delete_record(model, record_id)
                    .await?;
                Ok(Value::Null)
            }
            VmEffect::QueryRecords {
                model_id,
                limit,
                value,
            } => {
                let model = self.model_name(model_id)?;
                let offset = value
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize;
                serde_json::to_value(
                    self.data_store
                        .executor()
                        .list_records(
                            model,
                            PageParams {
                                o: offset,
                                s: limit as usize,
                            },
                        )
                        .await?,
                )
                .context("序列化查询记录结果失败")
            }
            VmEffect::Capability {
                capability_id,
                operation,
                value,
            } => {
                self.capabilities
                    .execute(&capability_id, &operation, value)
                    .await
            }
            VmEffect::ValidateForm { value, .. } => Ok(value),
            VmEffect::SetState { .. }
            | VmEffect::Navigate { .. }
            | VmEffect::Confirm { .. }
            | VmEffect::OpenDialog { .. }
            | VmEffect::CloseDialog { .. }
            | VmEffect::Notify { .. }
            | VmEffect::Refresh { .. }
            | VmEffect::InvokeServerSegment { .. } => {
                bail!("客户端 Effect 不允许在服务端 segment 中执行")
            }
        }
    }
}

impl ServerVmHost<'_> {
    fn model_name(&self, model_id: SymbolId) -> Result<&str> {
        self.image
            .models
            .get(&model_id)
            .map(|model| model.name.as_str())
            .with_context(|| format!("published model not found: {model_id}"))
    }
}

fn record_id(value: &Value) -> Result<&str> {
    value
        .as_str()
        .or_else(|| value.get("id").and_then(Value::as_str))
        .context("记录 Effect 缺少 id")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{ApplicationImage, CompiledRoute, ImageTarget, PROGRAM_SCHEMA_VERSION, SymbolId};

    use super::*;

    fn empty_image(revision_id: &str) -> ApplicationImage {
        ApplicationImage {
            schema_version: PROGRAM_SCHEMA_VERSION,
            compiler_version: PROGRAM_COMPILER_VERSION.to_owned(),
            content_hash: format!("hash-{revision_id}"),
            application_id: SymbolId::new(),
            name: "records".to_owned(),
            title: "记录".to_owned(),
            revision_id: revision_id.to_owned(),
            target: ImageTarget::Universal,
            menus: Vec::new(),
            permissions: Vec::new(),
            pages: BTreeMap::new(),
            client_functions: BTreeMap::new(),
            server_functions: BTreeMap::new(),
            models: BTreeMap::new(),
            routes: Vec::new(),
            dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn prebuilt_router_resolves_without_database() -> Result<()> {
        let route_id = SymbolId::new();
        let page_id = SymbolId::new();
        let mut image = empty_image("revision");
        image.routes = vec![CompiledRoute {
            id: route_id,
            name: "record".to_owned(),
            path: "/records/{id}".to_owned(),
            page_id,
            required_permissions: Vec::new(),
        }];
        let runtime = RuntimeApplicationImage::build(image)?;
        let matched = runtime.route("/records/42")?;
        assert_eq!(matched.route_id, route_id);
        assert_eq!(matched.parameters, vec![("id".to_owned(), "42".to_owned())]);
        Ok(())
    }

    #[test]
    fn in_flight_request_keeps_old_arc_after_atomic_swap() -> Result<()> {
        let old = Arc::new(RuntimeApplicationImage::build(empty_image("revision-old"))?);
        let slot = ArcSwapOption::new(Some(Arc::clone(&old)));
        let in_flight = slot.load_full().expect("请求应取得活动 Image");
        let new = Arc::new(RuntimeApplicationImage::build(empty_image("revision-new"))?);
        slot.store(Some(new));

        assert_eq!(in_flight.image().revision_id, "revision-old");
        assert_eq!(
            slot.load_full()
                .expect("发布后应存在新 Image")
                .image()
                .revision_id,
            "revision-new"
        );
        Ok(())
    }
}
