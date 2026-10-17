//! 挂载管理页面的交互与文件传输，不占用组件执行器。

use super::{frontend, model, services};
use crate::runtime::{
    RuntimeResponse,
    server::{RuntimeState, http_error::RuntimeError, request_context::authenticate},
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use az_plugin_contract::InvocationScope;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(super) struct Interaction {
    pub(super) state: RuntimeState,
    pub(super) scope: InvocationScope,
    pub(super) services: Arc<services::Services>,
    // 交互登记必须持续到正式管理处理完成，离开后立即失效。
    _authorization: services::Authorization,
}

pub(super) async fn interaction(
    state: &RuntimeState,
    headers: &HeaderMap,
    token: &str,
) -> Result<Interaction, RuntimeError> {
    let session = authenticate(state, headers).await?;
    let grant = frontend::validate(state, &session, token).await?;
    let components = state.components()?;
    let bundle = components
        .bundle(grant.source_id.parse()?, &session.tenant_id)
        .await?;
    let authorization = components
        .services
        .enter_frontend_management(&session, &grant.source_id)?;
    let scope = InvocationScope {
        source_id: grant.source_id,
        revision: grant.revision,
        context: authorization.context.clone(),
        grants: bundle.manifest().plugin.capabilities.clone(),
    };
    let state = components.management_state(&scope, &session).await?;
    Ok(Interaction {
        state,
        scope,
        services: components.services.clone(),
        _authorization: authorization,
    })
}

// 管理前端直接进入宿主契约，升级或停用自身不会等待自己的执行器请求排空。
pub(super) async fn manage(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(request): Json<model::Request>,
) -> Result<Json<RuntimeResponse<model::Response>>, RuntimeError> {
    let _permit = state.frontend.request_slot()?;
    let interaction = interaction(&state, &headers, &token).await?;
    let response = super::super::management_dispatch::invoke(
        interaction.state,
        interaction.scope,
        interaction.services,
        false,
        request.try_into()?,
    )
    .await?;
    Ok(Json(RuntimeResponse {
        data: response.into(),
    }))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum Action {
    DownloadPackage { revision: String },
    OpenSettings { page_id: String },
    InstallTool { id: String, version: String },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum ApprovedAction {
    DownloadPackage { revision: String },
    OpenSettings { page_id: String },
    InstallTool { url: String },
}

pub(super) async fn action(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(action): Json<Action>,
) -> Result<Json<RuntimeResponse<ApprovedAction>>, RuntimeError> {
    let _permit = state.frontend.request_slot()?;
    let interaction = interaction(&state, &headers, &token).await?;
    let action = match action {
        Action::DownloadPackage { revision } => {
            validate_revision(&revision)?;
            ApprovedAction::DownloadPackage { revision }
        }
        Action::OpenSettings { page_id } => {
            if page_id.len() > 512 {
                return Err(RuntimeError::bad_request("设置页面标识过长"));
            }
            let response = super::super::management_dispatch::invoke(
                interaction.state,
                interaction.scope,
                interaction.services,
                false,
                az_plugin_contract::transport::Request {
                    method: "GET".into(),
                    path: "/api/runtime/catalog".into(),
                    query: None,
                    headers: vec![],
                    body: vec![],
                },
            )
            .await?;
            if response.status != 200 {
                return Err(RuntimeError::forbidden("读取已授权设置目录失败"));
            }
            let catalog: serde_json::Value = serde_json::from_slice(&response.body)?;
            let pages = catalog["data"]["plugin_settings"]
                .as_array()
                .context("设置目录缺失")?;
            if !pages.iter().any(|page| page["page_id"] == page_id) {
                return Err(RuntimeError::forbidden("设置页面不可用或未授权"));
            }
            ApprovedAction::OpenSettings { page_id }
        }
        Action::InstallTool { id, version } => {
            let link = az_tool::InstallLink { id, version };
            az_tool::InstallLink::parse(&link.to_string())?;
            let response = super::super::management_dispatch::invoke(
                interaction.state,
                interaction.scope,
                interaction.services,
                false,
                az_plugin_contract::transport::Request {
                    method: "GET".into(),
                    path: format!("/api/runtime/tools/{}/{}", link.id, link.version),
                    query: None,
                    headers: vec![],
                    body: vec![],
                },
            )
            .await?;
            if response.status != 200 {
                return Err(RuntimeError::not_found("该工具版本或工具契约不可用"));
            }
            let manifest: az_tool::ToolManifest = serde_json::from_slice(&response.body)?;
            if manifest.id != link.id || manifest.version != link.version {
                return Err(RuntimeError::bad_request("工具版本与请求不符"));
            }
            ApprovedAction::InstallTool {
                url: link.to_string(),
            }
        }
    };
    Ok(Json(RuntimeResponse { data: action }))
}

// 浏览器直接接收正式下载响应，不经过 postMessage 或 JSON 数组，保留大包下载能力。
pub(super) async fn download(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((token, revision)): Path<(String, String)>,
) -> Result<Response, RuntimeError> {
    validate_revision(&revision)?;
    let _permit = state.frontend.request_slot()?;
    let interaction = interaction(&state, &headers, &token).await?;
    super::super::publication::download_package(State(interaction.state), headers, Path(revision))
        .await
}

fn validate_revision(revision: &str) -> Result<()> {
    ensure!(
        revision.len() == 64
            && revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "整包摘要无效"
    );
    Ok(())
}
