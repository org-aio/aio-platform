use axum::http::HeaderMap;
use az_plugin_contract::{
    CapabilityGrants, EnhancementRequest, EnhancementResponse, InvocationScope, RequestContext,
};

use super::super::{
    RuntimeState, enhancements::Enhancements, http_error::RuntimeError,
    request_context::authenticate,
};
use crate::identity::SessionContext;

/// 每个请求重新认证；传输层不能缓存上次请求的租户、角色或撤销状态。
pub(super) async fn session(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<SessionContext, RuntimeError> {
    let session = authenticate(state, headers).await?;
    if !state
        .identity
        .session_active(&session.session_id, &session.tenant_id, &session.user_id)
        .await?
    {
        return Err(RuntimeError::unauthorized("会话已撤销"));
    }
    Ok(session)
}

fn permitted(session: &SessionContext, path: &str) -> bool {
    let permission = format!("enhancement:{path}");
    session
        .permissions
        .iter()
        .any(|value| value == "plugin:manage" || value == &permission)
}

fn permitted_capability(session: &SessionContext, path: &str) -> bool {
    session
        .permissions
        .iter()
        .any(|value| value == &format!("runtime:{path}"))
}

pub(super) async fn catalog(
    state: &RuntimeState,
    session: &SessionContext,
) -> Result<(Option<String>, Vec<String>), RuntimeError> {
    let (revision, paths) = state
        .service::<Enhancements>()?
        .catalog(
            &session.tenant_id,
            |path| permitted(session, path),
            |path| permitted_capability(session, path),
        )
        .await?;
    Ok((revision, paths))
}

pub(super) async fn invoke(
    state: &RuntimeState,
    session: &SessionContext,
    request: EnhancementRequest,
) -> Result<EnhancementResponse, RuntimeError> {
    az_plugin_contract::validate_enhancement_path(&request.path)?;
    if !permitted(session, &request.path) {
        return Err(RuntimeError::forbidden("当前身份没有该增强能力权限"));
    }
    let (revision, grants) = catalog(state, session).await?;
    if revision.is_some() && !grants.contains(&request.path) {
        return Err(RuntimeError::forbidden("当前身份缺少增强依赖权限"));
    }
    // 外部用户调用明确标识为用户来源，不能伪装成已安装插件来借用包能力。
    let scope = InvocationScope {
        source_id: format!("user:{}", session.user_id),
        revision: String::new(),
        context: RequestContext {
            tenant_id: Some(session.tenant_id.clone()),
            user_id: Some(session.user_id.clone()),
            session_id: Some(session.session_id.clone()),
            request_id: uuid::Uuid::new_v4().to_string(),
        },
        grants: CapabilityGrants {
            enhancements: grants,
            runtime: session
                .permissions
                .iter()
                .filter_map(|permission| permission.strip_prefix("runtime:").map(str::to_owned))
                .collect(),
            ..Default::default()
        },
    };
    Ok(state
        .service::<Enhancements>()?
        .invoke(&scope, request)
        .await?)
}
