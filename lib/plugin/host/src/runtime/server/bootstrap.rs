use std::time::Instant;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use sha2::{Digest, Sha256};

use super::{RuntimeState, http_error::RuntimeError, request_context::catalog_value};
use crate::{
    runtime::RuntimeResponse,
    startup::{ApplicationSnapshot, LoadedApplication},
};

pub(super) fn router() -> Router<RuntimeState> {
    Router::new().route("/api/runtime/bootstrap", get(bootstrap))
}

async fn bootstrap(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Response, RuntimeError> {
    let (loaded, timing) = load(&state, &headers).await?;
    let etag = loaded.etag.as_deref().expect("已生成快照标识");
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag));
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        axum::Json(RuntimeResponse {
            data: &loaded.snapshot,
        })
        .into_response()
    };
    let output = response.headers_mut();
    output.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    output.insert(header::VARY, HeaderValue::from_static("Cookie"));
    output.insert(header::ETAG, HeaderValue::from_str(etag)?);
    output.insert("server-timing", timing);
    Ok(response)
}

pub(super) async fn load(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<(LoadedApplication, HeaderValue), RuntimeError> {
    let start = Instant::now();
    let session = state.identity.authenticate(&headers).await?;
    let authenticated = Instant::now();
    let snapshot = match session {
        Some(session) => Some(ApplicationSnapshot {
            catalog: catalog_value(state, &session).await?,
            permissions: session.permissions,
        }),
        None => None,
    };
    let body = serde_json::to_vec(&RuntimeResponse { data: &snapshot })?;
    let etag = format!("\"{:x}\"", Sha256::digest(&body));
    let timing = HeaderValue::from_str(&format!(
        "auth;dur={:.3}, catalog;dur={:.3}",
        authenticated.duration_since(start).as_secs_f64() * 1000.0,
        authenticated.elapsed().as_secs_f64() * 1000.0
    ))?;
    Ok((
        LoadedApplication {
            snapshot,
            etag: Some(etag),
        },
        timing,
    ))
}
