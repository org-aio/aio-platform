use std::{env, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use topcoat::{
    Result,
    context::Cx,
    router::{
        Router, RouterBuilderDiscoverExt,
        content::Json,
        request::headers,
        route,
    },
};

#[derive(Clone, Default)]
struct CounterState {
    value: Arc<Mutex<Counter>>,
}

#[derive(Clone, Default, Serialize)]
struct Counter {
    value: i64,
}

#[derive(Serialize)]
struct RuntimeContext {
    tenant_id: String,
    user_id: String,
}

#[derive(Deserialize)]
struct CounterRequest {
    #[serde(default)]
    increment: bool,
}

#[tokio::main]
async fn main() {
    if let Ok(port) = env::var("AIO_PLUGIN_PORT") {
        // Topcoat 使用 PORT；AIO 隔离进程只注入 AIO_PLUGIN_PORT。
        unsafe { env::set_var("PORT", port) };
    }
    topcoat::start(router()).await.unwrap();
}

fn router() -> Router {
    Router::builder()
        .app_context(CounterState::default())
        .discover()
        .build()
}

#[route(GET "/health")]
async fn health() -> Result<&'static str> {
    Ok("ok")
}

#[route(GET "/aio/describe")]
async fn describe() -> Result<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "label": "__TITLE__",
        "pages": [{
            "id": "__NAME__",
            "label": "__TITLE__",
            "entry": "index.html",
            "scene": ["workspace", "工作空间"],
            "menu_path": ["__TITLE__"],
            "permission": null,
            "surface": "workspace"
        }]
    })))
}

#[route(GET "/api/counter")]
async fn counter(cx: &Cx) -> Result<Json<Counter>> {
    let state = topcoat::context::app_context::<CounterState>(cx);
    let counter = state.value.lock().await.clone();
    Ok(Json(counter))
}

#[route(POST "/api/counter")]
async fn update_counter(cx: &Cx, Json(request): Json<CounterRequest>) -> Result<Json<Counter>> {
    let state = topcoat::context::app_context::<CounterState>(cx);
    let mut current = state.value.lock().await;
    if request.increment {
        current.value = current.value.saturating_add(1);
    }
    Ok(Json(current.clone()))
}

#[route(GET "/api/context")]
async fn context(cx: &Cx) -> Result<Json<RuntimeContext>> {
    let headers = headers(cx);
    Ok(Json(RuntimeContext {
        tenant_id: header(headers, "x-aio-tenant-id"),
        user_id: header(headers, "x-aio-user-id"),
    }))
}

fn header(headers: &topcoat::router::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use topcoat::{
        router::{Body, Method, StatusCode, request::Request, to_bytes},
    };

    async fn request(method: Method, path: &str, body: Body) -> (StatusCode, String) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("x-aio-tenant-id", "tenant-test")
            .header("x-aio-user-id", "user-test")
            .body(body)
            .unwrap();
        let response = router().handle(request).await;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn exposes_aio_runtime_contract() {
        let (status, body) = request(Method::GET, "/health", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");

        let (status, body) = request(Method::GET, "/aio/describe", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"id\":\"__NAME__\""));
        assert!(body.contains("\"entry\":\"index.html\""));
    }

    #[tokio::test]
    async fn shares_counter_and_preserves_host_context() {
        let (status, body) = request(Method::GET, "/api/counter", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "{\"value\":0}");

        let (status, body) = request(
            Method::POST,
            "/api/counter",
            Body::from("{\"increment\":true}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "{\"value\":1}");

        let (status, body) = request(Method::GET, "/api/context", Body::empty()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("tenant-test"));
        assert!(body.contains("user-test"));
    }
}
