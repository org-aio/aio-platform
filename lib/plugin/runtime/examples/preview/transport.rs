use super::Preview;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use az_plugin_contract::transport::{Header, Request};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRequest {
    method: String,
    path: String,
    query: Option<String>,
    headers: Vec<BrowserHeader>,
    body: Vec<u8>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserHeader {
    name: String,
    value: String,
}

#[derive(Serialize)]
pub struct BrowserResponse {
    status: u16,
    headers: Vec<BrowserHeader>,
    body: Vec<u8>,
}

pub async fn invoke(
    State(state): State<Arc<Preview>>,
    headers: HeaderMap,
    Json(request): Json<BrowserRequest>,
) -> Result<Json<BrowserResponse>, (StatusCode, String)> {
    if headers.get("origin").and_then(|value| value.to_str().ok()) != Some(&state.origin)
        || headers
            .get("x-aio-ticket")
            .and_then(|value| value.to_str().ok())
            != Some(&state.ticket)
    {
        return Err((StatusCode::FORBIDDEN, "Invalid preview mount".into()));
    }
    if !request.path.starts_with('/')
        || request.path.starts_with("//")
        || request.path.len() > 2048
        || request.headers.len() > 64
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid request".into()));
    }
    let request = Request {
        method: request.method,
        path: request.path,
        query: request.query,
        body: request.body,
        headers: request
            .headers
            .into_iter()
            .map(|header| Header {
                name: header.name,
                value: header.value,
            })
            .collect(),
    };
    let response = state
        .instance
        .lock()
        .await
        .handle(request, state.context.clone())
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("{error:#}")))?;
    Ok(Json(BrowserResponse {
        status: response.status,
        headers: response
            .headers
            .into_iter()
            .map(|header| BrowserHeader {
                name: header.name,
                value: header.value,
            })
            .collect(),
        body: response.body,
    }))
}
