use std::{sync::Arc, time::Duration};

use crate::{
    CompileFailure, CompiledArtifactWriter, GraphVm, GraphVmHost, ImageTarget, ProgramCompiler,
    ProgramDefinition, ProgramImage, RevisionRunSnapshot, RuntimeRecordInput, RuntimeRecordPage,
    RuntimeRecordView, SegmentInvocationRequest, SegmentInvocationResult, StudioPageParams,
    SymbolId, VmEffect,
};
use anyhow::{Context, Result, bail};
use arc_swap::ArcSwapOption;
use matchit::Router;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::postgres::PgListener;
use tokio::sync::{Mutex, broadcast};

use crate::{capability::CapabilityCatalog, program_store::ProgramStore};
use az_plugin_core::{
    PageParams, RecordCriteria, RecordFilter, RecordFilterOperator, RecordSort,
    RecordSortDirection, RecordStore,
};

pub const PROGRAM_COMPILER_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), ":program-v10");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProgramActivationEvent {
    pub revision_id: String,
    pub content_hash: String,
}

/// 运行时 Image 同时持有可序列化产物和预构建路由。
pub struct RuntimeProgramImage {
    image: ProgramImage,
    router: Router<SymbolId>,
}

impl RuntimeProgramImage {
    pub fn build(image: ProgramImage) -> Result<Self> {
        let mut router = Router::new();
        for route in &image.routes {
            router
                .insert(&route.path, route.id)
                .with_context(|| format!("预构建路由失败: {}", route.path))?;
        }
        Ok(Self { image, router })
    }

    #[must_use]
    pub fn image(&self) -> &ProgramImage {
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

fn runtime_record_view(record: az_plugin_core::DataRecordView) -> RuntimeRecordView {
    RuntimeRecordView {
        id: record.id,
        payload: record.payload,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRouteMatch {
    pub route_id: SymbolId,
    pub page_id: SymbolId,
    pub parameters: Vec<(String, String)>,
}

type ImageSlot = Arc<ArcSwapOption<RuntimeProgramImage>>;

/// 数据库程序的编译、发布、热切换与多实例同步入口。
#[derive(Clone)]
pub struct ProgramRuntime {
    store: ProgramStore,
    data_store: RecordStore,
    capability_catalog: Arc<CapabilityCatalog>,
    capabilities: Arc<crate::CapabilityCatalog>,
    compiled_artifacts: CompiledArtifactWriter,
    slot: ImageSlot,
    pending_generation: Arc<Mutex<u64>>,
    events: broadcast::Sender<ProgramActivationEvent>,
}

impl ProgramRuntime {
    #[must_use]
    pub fn new(
        store: ProgramStore,
        data_store: RecordStore,
        capability_catalog: CapabilityCatalog,
        compiled_artifacts: CompiledArtifactWriter,
    ) -> Self {
        let (events, _) = broadcast::channel(128);
        let capabilities = capability_catalog.program_catalog();
        Self {
            store,
            data_store,
            capability_catalog: Arc::new(capability_catalog),
            capabilities: Arc::new(capabilities),
            compiled_artifacts,
            slot: Arc::new(ArcSwapOption::empty()),
            pending_generation: Arc::new(Mutex::new(0)),
            events,
        }
    }

    #[must_use]
    pub fn store(&self) -> &ProgramStore {
        &self.store
    }

    #[must_use]
    pub fn capability_catalog(&self) -> &crate::CapabilityCatalog {
        &self.capabilities
    }

    pub fn validate_definition(
        &self,
        definition: &ProgramDefinition,
    ) -> Result<ProgramImage, CompileFailure> {
        ProgramCompiler::new(PROGRAM_COMPILER_VERSION, &self.capabilities).compile(
            definition,
            "validation",
            ImageTarget::Universal,
        )
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ProgramActivationEvent> {
        self.events.subscribe()
    }

    /// 请求入口取得 Arc 后可跨越发布过程安全使用旧 Image。
    pub async fn image(&self) -> Option<Arc<RuntimeProgramImage>> {
        self.slot.load_full()
    }

    pub async fn active_image(&self) -> Option<Arc<RuntimeProgramImage>> {
        self.image().await
    }

    pub async fn resolve_route(
        &self,
        path: &str,
    ) -> Result<(Arc<RuntimeProgramImage>, RuntimeRouteMatch)> {
        let image = self.image().await.context("活动 ProgramImage 不存在")?;
        let route = image.route(path)?;
        Ok((image, route))
    }

    pub async fn invoke_server_segment(
        &self,
        function_id: SymbolId,
        request: &SegmentInvocationRequest,
    ) -> Result<SegmentInvocationResult> {
        let image = self.image().await.context("活动 ProgramImage 不存在")?;
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

    pub async fn list_records(
        &self,
        model_id: SymbolId,
        page: StudioPageParams,
        criteria: &crate::RuntimeRecordCriteria,
    ) -> Result<RuntimeRecordPage> {
        let model_name = self.model_name(model_id).await?;
        let page_params = PageParams {
            o: page.o,
            s: page.s,
        };
        let executor = self.data_store.executor();
        let records = if criteria.is_empty() {
            executor.list_records(&model_name, page_params).await?
        } else {
            let criteria = engine_record_criteria(criteria);
            executor
                .list_records_with_criteria(&model_name, &criteria, page_params)
                .await?
        };
        Ok(RuntimeRecordPage {
            d: records.d.into_iter().map(runtime_record_view).collect(),
            t: records.t,
            p: page,
        })
    }

    pub async fn get_record(
        &self,
        model_id: SymbolId,
        record_id: &str,
    ) -> Result<RuntimeRecordView> {
        let model_name = self.model_name(model_id).await?;
        self.data_store
            .executor()
            .get_record(&model_name, record_id)
            .await
            .map(runtime_record_view)
    }

    pub async fn create_record(
        &self,
        model_id: SymbolId,
        input: RuntimeRecordInput,
    ) -> Result<RuntimeRecordView> {
        let model_name = self.model_name(model_id).await?;
        self.data_store
            .executor()
            .insert_record(&model_name, input.payload)
            .await
            .map(runtime_record_view)
    }

    pub async fn update_record(
        &self,
        model_id: SymbolId,
        record_id: &str,
        input: RuntimeRecordInput,
    ) -> Result<RuntimeRecordView> {
        let model_name = self.model_name(model_id).await?;
        self.data_store
            .executor()
            .update_record(&model_name, record_id, input.payload)
            .await
            .map(runtime_record_view)
    }

    pub async fn delete_record(&self, model_id: SymbolId, record_id: &str) -> Result<()> {
        let model_name = self.model_name(model_id).await?;
        self.data_store
            .executor()
            .delete_record(&model_name, record_id)
            .await
    }

    async fn model_name(&self, model_id: SymbolId) -> Result<String> {
        let image = self.image().await.context("活动 ProgramImage 不存在")?;
        image
            .image()
            .models
            .get(&model_id)
            .map(|model| model.name.clone())
            .with_context(|| format!("published model not found: {model_id}"))
    }

    pub async fn restore_active_image(&self) -> Result<()> {
        let Some(program) = self.store.active_program().await? else {
            return Ok(());
        };
        let revision_id = program
            .active_revision_id
            .context("活动 Program 缺少 Revision")?;
        self.load_revision(&revision_id, false)
            .await
            .with_context(|| format!("恢复活动 ProgramImage 失败: {}", program.id))?;
        Ok(())
    }

    pub async fn publish_draft_if_changed(&self, origin: &str) -> Result<bool> {
        let draft = self.store.draft().await?;
        let draft_hash =
            crate::content_hash(&draft.definition).context("计算 Draft 内容哈希失败")?;
        let program = self.store.program().await?;
        if let Some(revision_id) = program.active_revision_id {
            let revision = self.store.revision(&revision_id).await?;
            let active_hash = crate::content_hash(&revision.definition)
                .context("计算活动 Revision 内容哈希失败")?;
            if active_hash == draft_hash {
                return Ok(false);
            }
        }
        self.publish_latest(origin)
            .await
            .with_context(|| format!("发布最新 Program Draft 失败: {}", program.id))?;
        Ok(true)
    }

    /// 连续变更只保留 300ms 窗口内的最后一版。
    pub async fn schedule_publish(&self, origin: String) {
        let generation = {
            let mut pending = self.pending_generation.lock().await;
            *pending = pending.saturating_add(1);
            *pending
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let is_latest = {
                let pending = runtime.pending_generation.lock().await;
                *pending == generation
            };
            if !is_latest {
                return;
            }
            if let Err(error) = runtime.publish_latest(&origin).await {
                tracing::error!(
                    error = %format!("{error:#}"),
                    "自动发布 ProgramGraph 失败",
                );
            }
        });
    }

    pub async fn publish_latest(&self, origin: &str) -> Result<Arc<RuntimeProgramImage>> {
        let program_id = self.store.program().await?.id;
        let run = self.store.start_revision_run(&program_id).await?;
        let draft = self.store.draft().await?;
        let compiler = ProgramCompiler::new(PROGRAM_COMPILER_VERSION, &self.capabilities);
        let mut image = match compiler.compile(&draft.definition, "pending", ImageTarget::Universal)
        {
            Ok(image) => image,
            Err(failure) => {
                let diagnostics =
                    serde_json::to_value(&failure.diagnostics).context("序列化编译诊断失败")?;
                let revision = self
                    .store
                    .create_revision(&program_id, draft.definition, origin, &diagnostics)
                    .await?;
                self.finish_failed_run(&run, Some(&revision.id), "compile", &diagnostics)
                    .await?;
                return Err(failure.into());
            }
        };
        let diagnostics = Value::Array(Vec::new());
        let revision = self
            .store
            .create_revision(&program_id, draft.definition, origin, &diagnostics)
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
        image.program_id = revision.definition.id;
        let warmed = match RuntimeProgramImage::build(image.clone()) {
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
                return Err(error.context("ProgramImage 预热失败，活动版本保持不变"));
            }
        };

        if let Err(error) = self
            .store
            .reconcile_program_models(&program_id, &revision.definition, &image)
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

        self.compiled_artifacts
            .write(&image)
            .context("写入 Studio 编译产物失败")?;

        self.store.save_image(&image).await?;
        self.store
            .activate_revision(&program_id, &revision.id)
            .await?;
        self.swap_image(Arc::clone(&warmed)).await;
        let event = ProgramActivationEvent {
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
        revision_id: &str,
        emit_event: bool,
    ) -> Result<Arc<RuntimeProgramImage>> {
        let program = self.store.program().await?;
        if program.active_revision_id.as_deref() != Some(revision_id) {
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
            None => ProgramCompiler::new(PROGRAM_COMPILER_VERSION, &self.capabilities)
                .compile(&revision.definition, revision_id, ImageTarget::Universal)
                .map_err(anyhow::Error::from)?,
        };
        image.revision_id = revision_id.to_owned();
        image.program_id = revision.definition.id;
        self.store
            .reconcile_program_models(&program.id, &revision.definition, &image)
            .await
            .context("恢复活动 Revision 的动态模型元数据失败")?;
        self.compiled_artifacts
            .write(&image)
            .context("恢复活动 Revision 编译产物失败")?;
        self.store.save_image(&image).await?;
        let content_hash = image.content_hash.clone();
        let warmed = Arc::new(RuntimeProgramImage::build(image)?);
        self.swap_image(Arc::clone(&warmed)).await;
        if emit_event {
            let _ = self.events.send(ProgramActivationEvent {
                revision_id: revision_id.to_owned(),
                content_hash,
            });
        }
        Ok(warmed)
    }

    pub async fn activate_existing_revision(
        &self,
        revision_id: &str,
    ) -> Result<Arc<RuntimeProgramImage>> {
        let program_id = self.store.program().await?.id;
        let revision = self.store.revision(revision_id).await?;
        if revision.program_id != program_id {
            bail!("Revision 不属于当前 Program: {revision_id}");
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
            None => ProgramCompiler::new(PROGRAM_COMPILER_VERSION, &self.capabilities)
                .compile(&revision.definition, revision_id, ImageTarget::Universal)
                .map_err(anyhow::Error::from)?,
        };
        image.revision_id = revision_id.to_owned();
        image.program_id = revision.definition.id;
        self.store
            .reconcile_program_models(&program_id, &revision.definition, &image)
            .await
            .context("回滚 Revision 的动态模型元数据失败")?;
        self.compiled_artifacts
            .write(&image)
            .context("回滚 Revision 编译产物失败")?;
        self.store.save_image(&image).await?;
        let content_hash = image.content_hash.clone();
        let warmed = Arc::new(RuntimeProgramImage::build(image)?);
        self.store
            .activate_revision(&program_id, revision_id)
            .await?;
        self.swap_image(Arc::clone(&warmed)).await;
        let _ = self.events.send(ProgramActivationEvent {
            revision_id: revision_id.to_owned(),
            content_hash,
        });
        Ok(warmed)
    }

    /// 每个实例持有独立 LISTEN 连接，收到通知后从不可变 revision/cache 恢复。
    pub async fn spawn_postgres_listener(&self, database_url: &str) -> Result<()> {
        let runtime = self.clone();
        let database_url = database_url.to_owned();
        tokio::spawn(async move {
            let mut attempt = 1_u32;
            loop {
                let error = match runtime.listen_postgres_activations(&database_url).await {
                    Ok(()) => {
                        tracing::warn!("ProgramGraph LISTEN 任务意外结束，准备重连");
                        anyhow::anyhow!("ProgramGraph LISTEN 任务意外结束")
                    }
                    Err(error) => error,
                };
                let delay = postgres_listener_retry_delay(attempt);
                tracing::warn!(
                    error = %format!("{error:#}"),
                    retry_seconds = delay.as_secs(),
                    "ProgramGraph LISTEN 连接中断，等待重连",
                );
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
        });
        Ok(())
    }

    async fn listen_postgres_activations(&self, database_url: &str) -> Result<()> {
        let mut listener = PgListener::connect(database_url)
            .await
            .context("连接 ProgramGraph LISTEN 通道失败")?;
        listener
            .listen("engine_program_activated")
            .await
            .context("订阅 ProgramGraph 激活通知失败")?;
        loop {
            let notification = listener
                .recv()
                .await
                .context("接收 ProgramGraph 激活通知失败")?;
            let payload =
                match serde_json::from_str::<ActivationNotification>(notification.payload()) {
                    Ok(value) => value,
                    Err(error) => {
                        tracing::error!(error = %error, "解析 ProgramGraph 激活通知失败");
                        continue;
                    }
                };
            let already_loaded = self
                .image()
                .await
                .is_some_and(|image| image.image().revision_id == payload.revision_id);
            if already_loaded {
                continue;
            }
            if let Err(error) = self.load_revision(&payload.revision_id, true).await {
                tracing::error!(
                    revision_id = payload.revision_id,
                    error = %format!("{error:#}"),
                    "加载其他实例发布的 ProgramGraph 失败",
                );
            }
        }
    }

    async fn swap_image(&self, image: Arc<RuntimeProgramImage>) {
        self.slot.store(Some(image));
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

fn engine_record_criteria(value: &crate::RuntimeRecordCriteria) -> RecordCriteria {
    RecordCriteria {
        all: value.all.iter().map(engine_record_filter).collect(),
        any: value.any.iter().map(engine_record_filter).collect(),
        sort: value.sort.as_ref().map(|sort| RecordSort {
            field: sort.field.clone(),
            direction: match sort.direction {
                crate::RuntimeRecordSortDirection::Ascending => RecordSortDirection::Ascending,
                crate::RuntimeRecordSortDirection::Descending => RecordSortDirection::Descending,
            },
        }),
    }
}

fn engine_record_filter(value: &crate::RuntimeRecordFilter) -> RecordFilter {
    RecordFilter {
        field: value.field.clone(),
        operator: match value.operator {
            crate::RuntimeRecordFilterOperator::Equals => RecordFilterOperator::Equals,
            crate::RuntimeRecordFilterOperator::Contains => RecordFilterOperator::Contains,
        },
        value: value.value.clone(),
    }
}

#[derive(Deserialize)]
struct ActivationNotification {
    revision_id: String,
}

fn postgres_listener_retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(1_u64 << attempt.saturating_sub(1).min(5))
}

struct ServerVmHost<'a> {
    data_store: RecordStore,
    capabilities: Arc<CapabilityCatalog>,
    image: &'a ProgramImage,
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
            VmEffect::Navigate { .. }
            | VmEffect::Confirm { .. }
            | VmEffect::Notify { .. }
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
#[path = "program_runtime_tests.rs"]
mod tests;
