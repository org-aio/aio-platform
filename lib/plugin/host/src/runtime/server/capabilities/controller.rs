use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};

use super::{RuntimeCapabilities, RuntimeCapability};
use crate::{
    identity::IdentityProvider,
    runtime::server::{
        RuntimeState, http_error::RuntimeError, request_context::authenticate_manager,
    },
};

pub(in crate::runtime::server) fn router() -> Router<RuntimeState> {
    Router::new().route("/api/runtime/capabilities", get(catalog))
}

async fn catalog(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RuntimeCapability>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    if !state
        .service::<dyn IdentityProvider>()?
        .session_active(&session.session_id, &session.tenant_id, &session.user_id)
        .await?
    {
        return Err(RuntimeError::unauthorized("会话已撤销"));
    }
    Ok(Json(
        state
            .service::<RuntimeCapabilities>()?
            .catalog(&session.tenant_id)?,
    ))
}
