use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

use super::{
    RuntimeState, http_error::RuntimeError, remote_access::validate_git,
    request_context::authenticate_publish_manager,
};
use crate::runtime::{CreatePublishCredentialRequest, PublishCredentialView, RuntimeResponse};

pub(super) async fn create_publish_credential(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<CreatePublishCredentialRequest>,
) -> Result<Json<RuntimeResponse<PublishCredentialView>>, RuntimeError> {
    let session = authenticate_publish_manager(&state, &headers).await?;
    validate_git(&request.git)?;
    let credential = state
        .store
        .create_publish_credential(&session.tenant_id, &request.git)
        .await?;
    Ok(Json(RuntimeResponse { data: credential }))
}

pub(super) async fn revoke_publish_credential(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(credential_id): Path<String>,
) -> Result<StatusCode, RuntimeError> {
    let session = authenticate_publish_manager(&state, &headers).await?;
    state
        .store
        .revoke_publish_credential(&session.tenant_id, &credential_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
