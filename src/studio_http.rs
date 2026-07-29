//! 数据库原生 Studio 的 REST、SSE 与发布边界。

use std::{convert::Infallible, sync::Arc};

use crate::{
    ApplicationSummary, CreateApplicationInput, DraftSnapshot, GraphPatchBatch, PatchOrigin,
    ProgramPatchAgent, RevisionSnapshot, StudioPage, StudioPageParams, VibeMessageInput,
    VibeRunAccepted, VibeRunRequest,
    program_runtime::{ProgramActivationEvent, ProgramRuntime},
    program_store::DraftVersionConflict,
};
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

const APPLICATIONS_PATH: &str = "/api/studio/applications";
const CATALOG_PATH: &str = "/api/studio/catalog";
const DRAFT_PATH: &str = "/api/studio/applications/{application_id}/draft";
const REVISIONS_PATH: &str = "/api/studio/applications/{application_id}/revisions";
const ROLLBACK_PATH: &str =
    "/api/studio/applications/{application_id}/revisions/{revision_id}/rollback";
const EVENTS_PATH: &str = "/api/studio/applications/{application_id}/events";
const RUNTIME_IMAGE_PATH: &str = "/api/runtime/applications/{application_id}/image";
const SERVER_SEGMENT_PATH: &str =
    "/api/runtime/applications/{application_id}/segments/{function_id}";
const VIBE_RUNS_PATH: &str = "/api/studio/applications/{application_id}/vibe-runs";

#[derive(Clone)]
pub struct StudioState {
    runtime: Option<Arc<ProgramRuntime>>,
    patch_agent: Arc<ProgramPatchAgent>,
}

impl StudioState {
    #[must_use]
    pub fn new(runtime: Option<ProgramRuntime>, patch_agent: ProgramPatchAgent) -> Self {
        Self {
            runtime: runtime.map(Arc::new),
            patch_agent: Arc::new(patch_agent),
        }
    }

    fn runtime(&self) -> Result<Arc<ProgramRuntime>, ApiError> {
        self.runtime
            .clone()
            .ok_or_else(|| ApiError::service_unavailable("Studio 需要 PostgreSQL DATABASE_URL"))
    }
}

pub fn router(state: StudioState) -> Router {
    Router::new()
        .route(CATALOG_PATH, get(studio_catalog))
        .route(
            APPLICATIONS_PATH,
            get(list_applications).post(create_application),
        )
        .route(DRAFT_PATH, get(get_draft).patch(patch_draft))
        .route(REVISIONS_PATH, get(list_revisions))
        .route(ROLLBACK_PATH, post(rollback_revision))
        .route(EVENTS_PATH, get(application_events))
        .route(RUNTIME_IMAGE_PATH, get(runtime_image))
        .route(SERVER_SEGMENT_PATH, post(invoke_server_segment))
        .route(VIBE_RUNS_PATH, post(start_vibe_run))
        .with_state(state)
}

async fn studio_catalog(
    State(state): State<StudioState>,
) -> Result<Json<ApiResponse<crate::StudioCatalog>>, ApiError> {
    let runtime = state.runtime()?;
    Ok(ok_json(crate::StudioCatalog {
        components: runtime.component_catalog().clone(),
        capabilities: runtime.capability_catalog().clone(),
    }))
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct PaginationQuery {
    #[serde(default)]
    o: usize,
    #[serde(default = "default_page_size")]
    s: usize,
}

impl PaginationQuery {
    fn page(self) -> Result<StudioPageParams, ApiError> {
        if self.s == 0 || self.s > 200 {
            return Err(ApiError::bad_request("分页参数 s 必须在 1..=200"));
        }
        Ok(StudioPageParams {
            o: self.o,
            s: self.s,
        })
    }
}

const fn default_page_size() -> usize {
    50
}

async fn list_applications(
    State(state): State<StudioState>,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<ApiResponse<StudioPage<ApplicationSummary>>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .store()
        .list_applications(query.page()?)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn create_application(
    State(state): State<StudioState>,
    ApiJson(input): ApiJson<CreateApplicationInput>,
) -> Result<(StatusCode, Json<ApiResponse<ApplicationSummary>>), ApiError> {
    let runtime = state.runtime()?;
    let application = runtime
        .store()
        .create_application(input)
        .await
        .map_err(ApiError::from)?;
    runtime
        .schedule_publish(application.id.clone(), "studio".to_owned())
        .await;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse {
            code: StatusCode::CREATED.as_u16(),
            msg: "created".to_owned(),
            data: Some(application),
        }),
    ))
}

async fn get_draft(
    State(state): State<StudioState>,
    ApiPath(application_id): ApiPath<String>,
) -> Result<Json<ApiResponse<DraftSnapshot>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .store()
        .draft(&application_id)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn patch_draft(
    State(state): State<StudioState>,
    ApiPath(application_id): ApiPath<String>,
    ApiJson(batch): ApiJson<GraphPatchBatch>,
) -> Result<Json<ApiResponse<DraftSnapshot>>, ApiError> {
    let runtime = state.runtime()?;
    let origin = patch_origin(&batch.origin).to_owned();
    let draft = match runtime.store().patch_draft(&application_id, &batch).await {
        Ok(value) => value,
        Err(error) => {
            if let Some(conflict) = error.downcast_ref::<DraftVersionConflict>() {
                return Err(ApiError::new(StatusCode::CONFLICT, conflict.to_string()));
            }
            return Err(ApiError::bad_request(error.to_string()));
        }
    };
    runtime.schedule_publish(application_id, origin).await;
    Ok(ok_json(draft))
}

async fn list_revisions(
    State(state): State<StudioState>,
    ApiPath(application_id): ApiPath<String>,
    ApiQuery(query): ApiQuery<PaginationQuery>,
) -> Result<Json<ApiResponse<StudioPage<RevisionSnapshot>>>, ApiError> {
    let runtime = state.runtime()?;
    runtime
        .store()
        .revisions(&application_id, query.page()?)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn rollback_revision(
    State(state): State<StudioState>,
    ApiPath((application_id, revision_id)): ApiPath<(String, String)>,
) -> Result<Json<ApiResponse<RevisionSnapshot>>, ApiError> {
    let runtime = state.runtime()?;
    let revision = runtime
        .store()
        .rollback(&application_id, &revision_id)
        .await
        .map_err(ApiError::from)?;
    runtime
        .activate_existing_revision(&application_id, &revision.id)
        .await
        .map_err(ApiError::from)?;
    Ok(ok_json(revision))
}

async fn application_events(
    State(state): State<StudioState>,
    ApiPath(application_id): ApiPath<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let runtime = state.runtime()?;
    let mut receiver = runtime.subscribe();
    let events = async_stream::stream! {
        yield Ok(Event::default().event("ready").data(application_id.clone()));
        loop {
            loop {
                match receiver.recv().await {
                    Ok(event) if event.application_id == application_id => {
                        let payload = event_payload(&event);
                        let event = Event::default().event("activated").data(payload);
                        yield Ok(event);
                        break;
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    };
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

async fn runtime_image(
    State(state): State<StudioState>,
    ApiPath(application_id): ApiPath<String>,
) -> Result<Json<ApiResponse<crate::ApplicationImage>>, ApiError> {
    let runtime = state.runtime()?;
    let image = runtime
        .image(&application_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("active image not found: {application_id}")))?;
    Ok(ok_json(image.image().clone()))
}

async fn invoke_server_segment(
    State(state): State<StudioState>,
    ApiPath((application_id, function_id)): ApiPath<(String, String)>,
    ApiJson(request): ApiJson<crate::SegmentInvocationRequest>,
) -> Result<Json<ApiResponse<crate::SegmentInvocationResult>>, ApiError> {
    let runtime = state.runtime()?;
    let function_id = crate::SymbolId::parse(&function_id)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    runtime
        .invoke_server_segment(&application_id, function_id, &request)
        .await
        .map(ok_json)
        .map_err(ApiError::from)
}

async fn start_vibe_run(
    State(state): State<StudioState>,
    ApiPath(application_id): ApiPath<String>,
    ApiJson(request): ApiJson<VibeRunRequest>,
) -> Result<(StatusCode, Json<ApiResponse<VibeRunAccepted>>), ApiError> {
    if request.prompt.trim().is_empty() {
        return Err(ApiError::bad_request("Vibe prompt 不能为空"));
    }
    let runtime = state.runtime()?;
    let draft = runtime
        .store()
        .draft(&application_id)
        .await
        .map_err(ApiError::from)?;
    let session = runtime
        .store()
        .create_vibe_session(&application_id, draft.version)
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
    tokio::spawn(async move {
        if let Err(error) =
            run_vibe_agent(runtime.clone(), patch_agent, session, draft, request).await
        {
            tracing::error!(
                application_id,
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

async fn run_vibe_agent(
    runtime: Arc<ProgramRuntime>,
    patch_agent: Arc<crate::ProgramPatchAgent>,
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
                runtime.component_catalog(),
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
                    "message": error.to_string(),
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
                runtime
                    .store()
                    .patch_draft(&draft.application_id, &batch)
                    .await?;
                let image = runtime
                    .publish_latest(&draft.application_id, "vibe")
                    .await?;
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
        runtime
            .store()
            .patch_draft(
                &draft.application_id,
                &GraphPatchBatch {
                    base_version: draft.version,
                    patches: cumulative_patches,
                    origin: PatchOrigin::Vibe,
                },
            )
            .await?;
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
