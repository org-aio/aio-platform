use anyhow::{Context as _, Result};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse as _, Response},
};
use kuchikiki::traits::TendrilSink as _;

use super::RuntimeState;
use crate::startup::LoadedApplication;

pub async fn document(
    State(state): State<RuntimeState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::GET
        || request.uri().path().starts_with("/assets/")
        || request.uri().path().starts_with("/wasm/")
    {
        return next.run(request).await;
    }
    request.headers_mut().remove(header::IF_MODIFIED_SINCE);
    request.headers_mut().remove(header::IF_NONE_MATCH);
    let headers = request.headers().clone();
    let response = next.run(request).await;
    if response.status() != StatusCode::OK
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value.as_bytes().starts_with(b"text/html"))
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    parts
        .headers
        .append(header::VARY, HeaderValue::from_static("Cookie"));
    parts.headers.remove(header::LAST_MODIFIED);
    parts.headers.remove(header::ETAG);
    parts.headers.remove(header::CONTENT_LENGTH);
    let bytes = match to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    // 首页快照失败时仍交付壳，由有界的客户端请求提供重试入口。
    if let Ok(Ok((snapshot, timing))) = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        super::bootstrap::load(&state, &headers),
    )
    .await
        && let Ok(html) = render(&bytes, &snapshot)
    {
        parts.headers.insert("server-timing", timing);
        return Response::from_parts(parts, Body::from(html));
    }
    Response::from_parts(parts, Body::from(bytes))
}

fn render(html: &[u8], snapshot: &LoadedApplication) -> Result<Vec<u8>> {
    let document = kuchikiki::parse_html()
        .one(std::str::from_utf8(html)?)
        .document_node;
    let head = document
        .select_first("head")
        .map_err(|_| anyhow::anyhow!("首页缺少 head"))?;
    let fragment = kuchikiki::parse_html()
        .one("<script id='aio-startup-snapshot' type='application/json'></script>")
        .document_node;
    let script = fragment
        .select_first("script")
        .map_err(|_| anyhow::anyhow!("创建首页快照失败"))?;
    // JSON 脚本节点也不能包含可结束 script 的原始左尖括号。
    let text = serde_json::to_string(snapshot)?.replace('<', "\\u003c");
    script.as_node().append(kuchikiki::NodeRef::new_text(text));
    let node = script.as_node().clone();
    node.detach();
    head.as_node().prepend(node);
    let mut output = Vec::new();
    document.serialize(&mut output).context("序列化首页失败")?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_json_cannot_close_its_script_element() -> Result<()> {
        let snapshot = LoadedApplication {
            snapshot: None,
            etag: Some("</script><script>alert(1)</script>".to_owned()),
        };
        let html = render(
            b"<!doctype html><head><title>AIO</title></head><body></body>",
            &snapshot,
        )?;
        let document = kuchikiki::parse_html()
            .one(String::from_utf8(html)?)
            .document_node;
        assert_eq!(document.select("script").unwrap().count(), 1);
        let text = document
            .select_first("#aio-startup-snapshot")
            .unwrap()
            .text_contents();
        assert!(!text.contains('<'));
        let decoded: LoadedApplication = serde_json::from_str(&text)?;
        assert_eq!(decoded.etag, snapshot.etag);
        Ok(())
    }
}
