use axum::http::HeaderMap;
use az_plugin_contract::{
    CapabilityGrants, EnhancementRequest, EnhancementResponse, InvocationScope, RequestContext,
};
use az_plugin_enhancement::EnhancementContract;

use super::super::{
    RuntimeState, capabilities::CapabilityInvocation, enhancements::Enhancements,
    http_error::RuntimeError, request_context::authenticate,
};
use crate::identity::SessionContext;

/// 每个请求重新认证；传输层不能缓存上次请求的租户、角色或撤销状态。
pub(super) async fn session(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<SessionContext, RuntimeError> {
    let session = authenticate(state, headers).await?;
    if !state
        .service::<dyn crate::identity::IdentityProvider>()?
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
) -> Result<(Option<String>, Vec<EnhancementContract>), RuntimeError> {
    let Some(enhancements) = state.optional_service::<Enhancements>()? else {
        return Ok((None, vec![]));
    };
    let (revision, contracts) = enhancements
        .catalog(
            &session.tenant_id,
            |path| permitted(session, path),
            |path| permitted_capability(session, path),
        )
        .await?;
    Ok((revision, contracts))
}

pub(super) async fn invoke(
    state: &RuntimeState,
    session: &SessionContext,
    request: EnhancementRequest,
    deadline: Option<std::time::Instant>,
) -> Result<EnhancementResponse, RuntimeError> {
    az_plugin_contract::validate_enhancement_path(&request.path)?;
    if !permitted(session, &request.path) {
        return Err(RuntimeError::forbidden("当前身份没有该增强能力权限"));
    }
    let (revision, contracts) = catalog(state, session).await?;
    let grants: Vec<_> = contracts
        .into_iter()
        .map(|contract| contract.path)
        .collect();
    if revision.is_some() && !grants.contains(&request.path) {
        return Err(RuntimeError::forbidden("当前身份缺少增强依赖权限"));
    }
    let mut scope = scope(session);
    scope.grants.enhancements = grants;
    let enhancements = state
        .optional_service::<Enhancements>()?
        .ok_or_else(|| RuntimeError::not_found("宿主未安装增强服务"))?;
    Ok(enhancements.invoke(&scope, request, deadline).await?)
}

/// 原生入口的范围只取自本次认证结果，管理增强的权限不能扩大 runtime 授权。
pub(super) fn capability_invocation(
    state: &RuntimeState,
    session: &SessionContext,
    deadline: Option<std::time::Instant>,
) -> Result<CapabilityInvocation, RuntimeError> {
    Ok(state.capability_invocation(&scope(session), deadline)?)
}

pub(super) async fn invoke_capability(
    state: &RuntimeState,
    session: &SessionContext,
    request: EnhancementRequest,
    deadline: Option<std::time::Instant>,
) -> Result<serde_json::Value, RuntimeError> {
    az_plugin_contract::validate_enhancement_path(&request.path)?;
    if !permitted_capability(session, &request.path) {
        return Err(RuntimeError::forbidden("当前身份没有该宿主能力权限"));
    }
    let invocation = capability_invocation(state, session, deadline)?;
    if !invocation.available(&request.path) {
        return Err(RuntimeError::not_found("宿主能力不可见或不存在"));
    }
    Ok(invocation.invoke(&request.path, &request.input).await?)
}

fn scope(session: &SessionContext) -> InvocationScope {
    // 外部用户调用明确标识为用户来源，不能伪装成已安装插件来借用包能力。
    InvocationScope {
        source_id: format!("user:{}", session.user_id),
        revision: String::new(),
        context: RequestContext {
            tenant_id: Some(session.tenant_id.clone()),
            user_id: Some(session.user_id.clone()),
            session_id: Some(session.session_id.clone()),
            request_id: uuid::Uuid::new_v4().to_string(),
        },
        grants: CapabilityGrants {
            runtime: session
                .permissions
                .iter()
                .filter_map(|permission| permission.strip_prefix("runtime:").map(str::to_owned))
                .collect(),
            ..Default::default()
        },
    }
}
