//! 数据库原生 Studio 的 REST、SSE 与发布边界。

use std::{convert::Infallible, sync::Arc};

use crate::studio_contract::StudioPage as StudioPageData;
use crate::{
    ApplicationBundle, ApplicationCompiler, ApplicationGenerationResult, ApplicationWorkspace,
    BusinessModuleManager, DraftSnapshot, FormStateExtractionRequest, FormStateExtractionResponse,
    FormStateExtractor, GraphPatchBatch, PatchOrigin, ProgramDefinition, ProgramPatchAgent,
    RevisionSnapshot, RuntimeRecordCriteria, RuntimeRecordInput, RuntimeRecordPage,
    RuntimeRecordView, StudioPageParams, VibeMessageInput, VibeRunAccepted, VibeRunRequest,
    VibeSessionSnapshot,
    program_runtime::{ProgramActivationEvent, ProgramRuntime},
    program_store::DraftVersionConflict,
};
use anyhow::Context as _;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use az_plugin_core::http::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResponse, ok_json};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use uuid::Uuid;

const CATALOG_PATH: &str = "/api/studio/catalog";
const DRAFT_PATH: &str = "/api/studio/program/draft";
const REVISIONS_PATH: &str = "/api/studio/program/revisions";
const ROLLBACK_PATH: &str = "/api/studio/program/revisions/{revision_id}/rollback";
const EVENTS_PATH: &str = "/api/studio/program/events";
const RUNTIME_IMAGE_PATH: &str = "/api/runtime/program/image";
const RUNTIME_DEFINITION_PATH: &str = "/api/runtime/program/definition";
const APPLICATION_PREVIEW_PATH: &str = "/api/studio/application/preview";
const APPLICATION_GENERATE_PATH: &str = "/api/studio/application/generate";
const SERVER_SEGMENT_PATH: &str = "/api/runtime/program/segments/{function_id}";
const VIBE_RUNS_PATH: &str = "/api/studio/program/vibe-runs";
const VIBE_RUN_PATH: &str = "/api/studio/program/vibe-runs/{session_id}";
const RUNTIME_RECORDS_PATH: &str = "/api/runtime/models/{model_id}/records";
const RUNTIME_RECORD_PATH: &str = "/api/runtime/models/{model_id}/records/{record_id}";
const FORM_STATE_EXTRACTION_PATH: &str = "/api/runtime/models/{model_id}/form-state/extract";

#[derive(Clone)]
pub struct StudioState {
    runtime: Option<Arc<ProgramRuntime>>,
    patch_agent: Arc<ProgramPatchAgent>,
    form_state_extractor: Arc<FormStateExtractor>,
    business_modules: Arc<BusinessModuleManager>,
}

impl StudioState {
    #[must_use]
    pub fn new(
        runtime: Option<Arc<ProgramRuntime>>,
        patch_agent: Arc<ProgramPatchAgent>,
        form_state_extractor: Arc<FormStateExtractor>,
        business_modules: Arc<BusinessModuleManager>,
    ) -> Self {
        Self {
            runtime,
            patch_agent,
            form_state_extractor,
            business_modules,
        }
    }

    fn runtime(&self) -> Result<Arc<ProgramRuntime>, ApiError> {
        self.runtime.clone().ok_or_else(|| {
            ApiError::service_unavailable(
                "Studio 当前未配置 PostgreSQL；请设置 AZ_AIO_DATABASE_URL 后重启 AIO",
            )
        })
    }
}

pub fn router(state: StudioState) -> Router {
    Router::new()
        .route(CATALOG_PATH, get(studio_catalog))
        .route(DRAFT_PATH, get(get_draft).patch(patch_draft))
        .route(REVISIONS_PATH, get(list_revisions))
        .route(ROLLBACK_PATH, post(rollback_revision))
        .route(EVENTS_PATH, get(program_events))
        .route(RUNTIME_IMAGE_PATH, get(runtime_image))
        .route(RUNTIME_DEFINITION_PATH, get(runtime_definition))
        .route(APPLICATION_PREVIEW_PATH, get(preview_application))
        .route(APPLICATION_GENERATE_PATH, post(generate_application))
        .route(SERVER_SEGMENT_PATH, post(invoke_server_segment))
        .route(VIBE_RUNS_PATH, post(start_vibe_run))
        .route(VIBE_RUN_PATH, get(get_vibe_run))
        .route(FORM_STATE_EXTRACTION_PATH, post(extract_form_state))
        .route(
            RUNTIME_RECORDS_PATH,
            get(list_runtime_records).post(create_runtime_record),
        )
        .route(
            RUNTIME_RECORD_PATH,
            get(get_runtime_record)
                .patch(update_runtime_record)
                .delete(delete_runtime_record),
        )
        .with_state(state)
}

async fn extract_form_state(
    State(state): State<StudioState>,
    ApiPath(model_id): ApiPath<String>,
    ApiJson(request): ApiJson<FormStateExtractionRequest>,
) -> Result<Json<ApiResponse<FormStateExtractionResponse>>, ApiError> {
    if request.prompt.trim().is_empty() {
        return Err(ApiError::bad_request("AI 表单输入不能为空"));
    }
    if !request.current_form_state.is_object() {
        return Err(ApiError::bad_request("current_form_state 必须是 JSON 对象"));
    }
    let model_id = parse_symbol_id(&model_id)?;
    let runtime = state.runtime()?;
    let image = runtime
        .image()
        .ok_or_else(|| ApiError::not_found("活动 ProgramImage 不存在"))?;
    let model = image
        .image()
        .models
        .get(&model_id)
        .ok_or_else(|| ApiError::not_found(format!("模型不存在: {model_id}")))?;
    match state
        .form_state_extractor
        .extract(
            model,
            request.prompt.trim(),
            &request.current_form_state,
            request.model.as_deref(),
        )
        .await
    {
        Ok(response) => Ok(ok_json(response)),
        Err(error) => {
            tracing::error!(
                model_id = %model_id,
                error = %format!("{error:#}"),
                "AI formState 提取失败",
            );
            Err(ApiError::internal("AI formState 提取失败"))
        }
    }
}

async fn studio_catalog(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<crate::StudioCatalog>>, ApiError> {
    let runtime = state.runtime()?;
    Ok(ok_json(crate::StudioCatalog {
        capabilities: runtime.capability_catalog().clone(),
    }))
}

#[derive(Clone, Debug, Deserialize)]
struct PaginationQuery {
    #[serde(default)]
    o: usize,
    #[serde(default = "default_page_size")]
    s: usize,
    criteria: Option<String>,
}

impl PaginationQuery {
    fn page(&self) -> Result<StudioPageParams, ApiError> {
        if self.s == 0 || self.s > 200 {
            return Err(ApiError::bad_request("分页参数 s 必须在 1..=200"));
        }
        Ok(StudioPageParams {
            o: self.o,
            s: self.s,
        })
    }

    fn criteria(&self) -> Result<RuntimeRecordCriteria, ApiError> {
        let Some(raw) = self.criteria.as_deref() else {
            return Ok(RuntimeRecordCriteria::default());
        };
        let mut criteria = serde_json::from_str::<RuntimeRecordCriteria>(raw)
            .map_err(|error| ApiError::bad_request(format!("记录查询条件无效: {error}")))?;
        if criteria.all.len().saturating_add(criteria.any.len()) > 16 {
            return Err(ApiError::bad_request("记录查询条件不能超过 16 个"));
        }
        for filter in criteria.all.iter_mut().chain(&mut criteria.any) {
            filter.field = filter.field.trim().to_owned();
            if filter.field.is_empty() || filter.value.is_empty() {
                return Err(ApiError::bad_request("记录筛选字段和值不能为空"));
            }
        }
        if let Some(sort) = criteria.sort.as_mut() {
            sort.field = sort.field.trim().to_owned();
            if sort.field.is_empty() {
                return Err(ApiError::bad_request("记录排序字段不能为空"));
            }
        }
        Ok(criteria)
    }
}

const fn default_page_size() -> usize {
    50
}

async fn get_draft(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<DraftSnapshot>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .store()
        .draft()
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn patch_draft(
    State(state): State<StudioState>,
    ApiJson(batch): ApiJson<GraphPatchBatch>,
) -> Result<Json<ApiResponse<DraftSnapshot>>, ApiError> {
    let runtime = state.runtime()?;
    let origin = patch_origin(&batch.origin).to_owned();
    let draft = match runtime.store().patch_draft(&batch).await {
        Ok(value) => value,
        Err(error) => {
            if let Some(conflict) = error.downcast_ref::<DraftVersionConflict>() {
                return Err(ApiError::new(StatusCode::CONFLICT, conflict.to_string()));
            }
            return Err(ApiError::bad_request(format!("{error:#}")));
        }
    };
    reconcile_generated_sources(&state.business_modules, &draft.definition)
        .map_err(ApiError::from)?;
    runtime.schedule_publish(origin).await;
    Ok(ok_json(draft))
}

async fn list_runtime_records(
    State(state): State<StudioState>,
    ApiPath(model_id): ApiPath<String>,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<ApiResponse<RuntimeRecordPage>>, ApiError> {
    let runtime = state.runtime()?;
    let criteria = query.criteria()?;
    runtime
        .list_records(parse_symbol_id(&model_id)?, query.page()?, &criteria)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn get_runtime_record(
    State(state): State<StudioState>,
    ApiPath((model_id, record_id)): ApiPath<(String, String)>,
) -> Result<Json<ApiResponse<RuntimeRecordView>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .get_record(parse_symbol_id(&model_id)?, &record_id)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn create_runtime_record(
    State(state): State<StudioState>,
    ApiPath(model_id): ApiPath<String>,
    ApiJson(input): ApiJson<RuntimeRecordInput>,
) -> Result<Json<ApiResponse<RuntimeRecordView>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .create_record(parse_symbol_id(&model_id)?, input)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn update_runtime_record(
    State(state): State<StudioState>,
    ApiPath((model_id, record_id)): ApiPath<(String, String)>,
    ApiJson(input): ApiJson<RuntimeRecordInput>,
) -> Result<Json<ApiResponse<RuntimeRecordView>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .update_record(parse_symbol_id(&model_id)?, &record_id, input)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn delete_runtime_record(
    State(state): State<StudioState>,
    ApiPath((model_id, record_id)): ApiPath<(String, String)>,
) -> Result<Json<ApiResponse<bool>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .delete_record(parse_symbol_id(&model_id)?, &record_id)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_json(true))
}

fn parse_symbol_id(value: &str) -> Result<crate::SymbolId, ApiError> {
    crate::SymbolId::parse(value).map_err(|error| ApiError::bad_request(error.to_string()))
}

async fn list_revisions(
    State(state): State<StudioState>,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<ApiResponse<StudioPageData<RevisionSnapshot>>>, ApiError> {
    let runtime = state.runtime()?;
    let program_id = runtime.store().program().await.map_err(ApiError::from)?.id;
    runtime
        .store()
        .revisions(&program_id, query.page()?)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn rollback_revision(
    State(state): State<StudioState>,
    ApiPath(revision_id): ApiPath<String>,
) -> Result<Json<ApiResponse<RevisionSnapshot>>, ApiError> {
    let runtime = state.runtime()?;
    let program_id = runtime.store().program().await.map_err(ApiError::from)?.id;
    let revision = runtime
        .store()
        .rollback(&program_id, &revision_id)
        .await
        .map_err(ApiError::from)?;
    let draft = runtime.store().draft().await.map_err(ApiError::from)?;
    reconcile_generated_sources(&state.business_modules, &draft.definition)
        .map_err(ApiError::from)?;
    runtime
        .activate_existing_revision(&revision.id)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_json(revision))
}

async fn program_events(
    State(state): State<StudioState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let runtime = state.runtime()?;
    let mut receiver = runtime.subscribe();
    let events = async_stream::stream! {
        yield Ok(Event::default().event("ready").data("program"));
        loop {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let payload = event_payload(&event);
                        let event = Event::default().event("activated").data(payload);
                        yield Ok(event);
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    };
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

async fn runtime_image(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<crate::ProgramImage>>, ApiError> {
    let runtime = state.runtime()?;
    let image = runtime
        .image()
        .ok_or_else(|| ApiError::not_found("活动 ProgramImage 不存在"))?;
    Ok(ok_json(image.image().clone()))
}

async fn runtime_definition(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<ProgramDefinition>>, ApiError> {
    let runtime = state.runtime()?;
    let program = runtime.store().program().await.map_err(ApiError::from)?;
    let revision_id = program
        .active_revision_id
        .ok_or_else(|| ApiError::not_found("活动 Program Revision 不存在"))?;
    runtime
        .store()
        .revision(&revision_id)
        .await
        .map(|revision| ok_json(revision.definition))
        .map_err(ApiError::from)
}

async fn preview_application(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<ApplicationBundle>>, ApiError> {
    let runtime = state.runtime()?;
    let draft = runtime.store().draft().await.map_err(ApiError::from)?;
    let mut image = runtime
        .validate_definition(&draft.definition)
        .map_err(anyhow::Error::from)
        .map_err(ApiError::from)?;
    image.revision_id = format!("draft-{}", draft.version);
    ApplicationCompiler
        .compile(&draft.definition, &image)
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn generate_application(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<ApplicationGenerationResult>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .publish_draft_if_changed("application-generation")
        .await
        .map_err(ApiError::from)?;
    let image = runtime
        .image()
        .ok_or_else(|| ApiError::not_found("活动 ProgramImage 不存在"))?;
    let revision = runtime
        .store()
        .revision(&image.image().revision_id)
        .await
        .map_err(ApiError::from)?;
    state
        .business_modules
        .reconcile(&revision.definition)
        .map_err(ApiError::from)?;
    let bundle = ApplicationCompiler
        .compile(&revision.definition, image.image())
        .map_err(ApiError::from)?;
    tokio::task::spawn_blocking(move || ApplicationWorkspace::repository().write(&bundle))
        .await
        .map_err(|error| ApiError::internal(format!("生成应用任务失败: {error}")))?
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn invoke_server_segment(
    State(state): State<StudioState>,
    ApiPath(function_id): ApiPath<String>,
    ApiJson(request): ApiJson<crate::SegmentInvocationRequest>,
) -> Result<Json<ApiResponse<crate::SegmentInvocationResult>>, ApiError> {
    let runtime = state.runtime()?;
    let function_id = crate::SymbolId::parse(&function_id)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    runtime
        .invoke_server_segment(function_id, &request)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn start_vibe_run(
    State(state): State<StudioState>,
    ApiJson(request): ApiJson<VibeRunRequest>,
) -> Result<(StatusCode, Json<ApiResponse<VibeRunAccepted>>), ApiError> {
    if request.prompt.trim().is_empty() {
        return Err(ApiError::bad_request("Vibe prompt 不能为空"));
    }
    let runtime = state.runtime()?;
    let draft = runtime.store().draft().await.map_err(ApiError::from)?;
    let session = runtime
        .store()
        .create_vibe_session(&draft.program_id, draft.version)
        .await
        .map_err(ApiError::from)?;
    runtime
        .store()
        .append_vibe_message(
            &session.id,
            &VibeMessageInput {
                role: "user".to_owned(),
                prompt: request.prompt.clone(),
                model: request.model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                patch: None,
                diagnostics: Value::Array(Vec::new()),
                tests: Value::Array(Vec::new()),
            },
        )
        .await
        .map_err(ApiError::from)?;
    let session_id = session.id.clone();
    let failure_session_id = session.id.clone();
    let patch_agent = Arc::clone(&state.patch_agent);
    let business_modules = state.business_modules.clone();
    tokio::spawn(async move {
        if let Err(error) = run_vibe_agent(
            runtime.clone(),
            patch_agent,
            business_modules,
            session,
            draft,
            request,
        )
        .await
        {
            tracing::error!(
                error = %format!("{error:#}"),
                "ProgramPatchAgent 执行失败",
            );
            if let Err(finish_error) = runtime
                .store()
                .finish_vibe_session(&failure_session_id, None, false)
                .await
            {
                tracing::error!(
                    session_id = failure_session_id,
                    error = %format!("{finish_error:#}"),
                    "标记 Vibe Session 失败状态失败",
                );
            }
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            code: StatusCode::ACCEPTED.as_u16(),
            msg: "accepted".to_owned(),
            data: Some(VibeRunAccepted {
                session_id,
                status: "running".to_owned(),
            }),
        }),
    ))
}

async fn get_vibe_run(
    State(state): State<StudioState>,
    ApiPath(session_id): ApiPath<String>,
) -> Result<Json<ApiResponse<VibeSessionSnapshot>>, ApiError> {
    Uuid::parse_str(&session_id).map_err(|_| ApiError::bad_request("Vibe Session ID 无效"))?;
    let runtime = state.runtime()?;
    let session = runtime
        .store()
        .vibe_session(&session_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found("Vibe Session 不存在"))?;
    Ok(ok_json(session))
}

async fn run_vibe_agent(
    runtime: Arc<ProgramRuntime>,
    patch_agent: Arc<crate::ProgramPatchAgent>,
    business_modules: Arc<BusinessModuleManager>,
    session: crate::VibeSessionSnapshot,
    draft: DraftSnapshot,
    request: VibeRunRequest,
) -> anyhow::Result<()> {
    let mut candidate = draft.definition.clone();
    let mut cumulative_patches = Vec::new();
    let mut diagnostics = Vec::<Value>::new();
    for attempt in 0..=2 {
        let generated = match patch_agent
            .generate(
                &request.prompt,
                request.model.as_deref(),
                draft.version,
                &candidate,
                runtime.capability_catalog(),
                &diagnostics,
            )
            .await
        {
            Ok(value) => value,
            Err(error) => {
                diagnostics = vec![json!({
                    "code": "VIBE_INFERENCE_FAILED",
                    "attempt": attempt,
                    "message": vibe_error_message(&error),
                })];
                record_vibe_gate(&runtime, &session.id, attempt, &diagnostics).await?;
                continue;
            }
        };
        runtime
            .store()
            .append_vibe_message(
                &session.id,
                &VibeMessageInput {
                    role: "agent".to_owned(),
                    prompt: format!("attempt {attempt}"),
                    model: Some(generated.model.clone()),
                    input_tokens: token_count(generated.input_tokens),
                    output_tokens: token_count(generated.output_tokens),
                    patch: Some(serde_json::to_value(&generated.batch)?),
                    diagnostics: Value::Array(Vec::new()),
                    tests: Value::Array(Vec::new()),
                },
            )
            .await?;
        match candidate.apply_patch_batch(&generated.batch) {
            Ok(()) => cumulative_patches.extend(generated.batch.patches),
            Err(error) => {
                diagnostics = vec![json!({
                    "code": "VIBE_PATCH_INVALID",
                    "attempt": attempt,
                    "message": error.to_string(),
                })];
                record_vibe_gate(&runtime, &session.id, attempt, &diagnostics).await?;
                continue;
            }
        }
        match runtime.validate_definition(&candidate) {
            Ok(_) => {
                let batch = GraphPatchBatch {
                    base_version: draft.version,
                    patches: cumulative_patches,
                    origin: PatchOrigin::Vibe,
                };
                let draft = runtime.store().patch_draft(&batch).await?;
                reconcile_generated_sources(&business_modules, &draft.definition)?;
                let image = runtime.publish_latest("vibe").await?;
                runtime
                    .store()
                    .finish_vibe_session(&session.id, Some(&image.image().revision_id), true)
                    .await?;
                return Ok(());
            }
            Err(failure) => {
                diagnostics = serde_json::to_value(&failure.diagnostics)?
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                record_vibe_gate(&runtime, &session.id, attempt, &diagnostics).await?;
            }
        }
    }

    if !cumulative_patches.is_empty() {
        let draft = runtime
            .store()
            .patch_draft(&GraphPatchBatch {
                base_version: draft.version,
                patches: cumulative_patches,
                origin: PatchOrigin::Vibe,
            })
            .await?;
        reconcile_generated_sources(&business_modules, &draft.definition)?;
    }
    runtime
        .store()
        .finish_vibe_session(&session.id, None, false)
        .await?;
    Ok(())
}

async fn record_vibe_gate(
    runtime: &ProgramRuntime,
    session_id: &str,
    attempt: usize,
    diagnostics: &[Value],
) -> anyhow::Result<()> {
    runtime
        .store()
        .append_vibe_message(
            session_id,
            &VibeMessageInput {
                role: "gate".to_owned(),
                prompt: format!("attempt {attempt}"),
                model: None,
                input_tokens: 0,
                output_tokens: 0,
                patch: None,
                diagnostics: Value::Array(diagnostics.to_vec()),
                tests: Value::Array(Vec::new()),
            },
        )
        .await?;
    Ok(())
}

fn token_count(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn reconcile_generated_sources(
    business_modules: &BusinessModuleManager,
    definition: &ProgramDefinition,
) -> anyhow::Result<()> {
    business_modules
        .reconcile(definition)
        .context("同步业务模块生成源码失败")?;
    ApplicationWorkspace::repository()
        .reconcile_page_sources(definition)
        .context("同步生成应用页面源码失败")
}

fn vibe_error_message(error: &anyhow::Error) -> String {
    let context = error.to_string();
    let root_cause = error.root_cause().to_string();
    if context == root_cause {
        return context;
    }
    format!("{context}: {root_cause}")
}

fn event_payload(event: &ProgramActivationEvent) -> String {
    match serde_json::to_string(event) {
        Ok(value) => value,
        Err(error) => format!(r#"{{"code":"EVENT_SERIALIZE_FAILED","msg":"{error}"}}"#),
    }
}

const fn patch_origin(origin: &PatchOrigin) -> &'static str {
    match origin {
        PatchOrigin::Studio => "studio",
        PatchOrigin::Vibe => "vibe",
        PatchOrigin::Migration => "migration",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn vibe_error_message_keeps_context_and_root_cause_once() {
        let result = Err::<(), _>(anyhow::anyhow!("Invalid status code 502 Bad Gateway"))
            .context("ProgramPatchAgent Responses 调用失败: gpt-5.5");
        let error = match result {
            Ok(()) => panic!("带上下文的错误必须保持失败状态"),
            Err(error) => error,
        };

        assert_eq!(
            vibe_error_message(&error),
            "ProgramPatchAgent Responses 调用失败: gpt-5.5: Invalid status code 502 Bad Gateway",
        );
    }

    #[test]
    fn record_criteria_is_structured_and_bounded() {
        let complete = PaginationQuery {
            o: 0,
            s: 20,
            criteria: Some(
                serde_json::json!({
                    "all": [{
                        "field": "department_id",
                        "operator": "equals",
                        "value": "department-1"
                    }]
                })
                .to_string(),
            ),
        };
        assert!(complete.criteria().is_ok());
        let incomplete = PaginationQuery {
            criteria: Some("{\"all\":[{}]}".to_owned()),
            ..complete
        };

        assert!(incomplete.criteria().is_err());
    }
}
