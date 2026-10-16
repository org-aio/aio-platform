use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::http::{HeaderName, HeaderValue};
use futures_util::{StreamExt, TryStreamExt, stream::BoxStream};
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::streamable_http_client::{
        StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use sse_stream::{Sse, SseStream};

use super::configuration::{CONNECT_TIMEOUT, MAX_WIRE_BYTES};

type Error = StreamableHttpError<reqwest::Error>;

/// SDK 负责 MCP 协议，此 HTTP 后端只增加读取限额、禁止重定向与固定认证头。
#[derive(Clone)]
pub(super) struct BoundedClient {
    client: reqwest::Client,
}

impl BoundedClient {
    pub fn new(auth: Option<HeaderValue>) -> anyhow::Result<Self> {
        let mut headers = axum::http::HeaderMap::new();
        if let Some(auth) = auth {
            headers.insert("authorization", auth);
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .default_headers(headers)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(Duration::from_secs(30))
                .build()?,
        })
    }

    fn request(
        &self,
        method: reqwest::Method,
        uri: &str,
        session: Option<&str>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<reqwest::RequestBuilder, Error> {
        let mut request = self
            .client
            .request(method, uri)
            .header("accept", "application/json, text/event-stream");
        for (name, value) in headers {
            if matches!(
                name.as_str(),
                "authorization"
                    | "cookie"
                    | "host"
                    | "mcp-session-id"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "proxy-authorization"
                    | "upgrade"
                    | "te"
                    | "trailer"
                    | "keep-alive"
                    | "accept"
                    | "content-type"
                    | "last-event-id"
            ) || name.as_str().starts_with("x-aio-")
            {
                return Err(Error::ReservedHeaderConflict(name.to_string()));
            }
            request = request.header(name, value);
        }
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        Ok(request)
    }
}

impl StreamableHttpClient for BoundedClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        _: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, Error> {
        let notification = !matches!(message, ClientJsonRpcMessage::Request(_));
        let response = self
            .request(reqwest::Method::POST, &uri, session_id.as_deref(), headers)?
            .json(&message)
            .send()
            .await
            .map_err(Error::Client)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND && session_id.is_some() {
            return Err(Error::SessionExpired);
        }
        let response = response.error_for_status().map_err(Error::Client)?;
        if !response.status().is_success() {
            return Err(Error::UnexpectedServerResponse(
                format!("MCP HTTP {}", response.status()).into(),
            ));
        }
        if matches!(
            response.status(),
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if content_type.starts_with("text/event-stream") {
            return Ok(StreamableHttpPostResponse::Sse(events(response), session));
        }
        if !content_type.starts_with("application/json") {
            return Err(Error::UnexpectedContentType(Some(content_type)));
        }
        let mut bytes = Vec::new();
        let mut stream = bounded(response);
        while let Some(chunk) = stream.try_next().await? {
            bytes.extend_from_slice(&chunk);
        }
        if notification && bytes.is_empty() {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let message: ServerJsonRpcMessage = serde_json::from_slice(&bytes)?;
        Ok(StreamableHttpPostResponse::Json(message, session))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        _: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), Error> {
        let response = self
            .request(reqwest::Method::DELETE, &uri, Some(&session_id), headers)?
            .send()
            .await
            .map_err(Error::Client)?;
        if matches!(
            response.status(),
            reqwest::StatusCode::METHOD_NOT_ALLOWED | reqwest::StatusCode::NOT_FOUND
        ) {
            return Ok(());
        }
        response.error_for_status().map_err(Error::Client)?;
        Ok(())
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        _: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, sse_stream::Error>>, Error> {
        let mut request =
            self.request(reqwest::Method::GET, &uri, session_id.as_deref(), headers)?;
        if let Some(last) = last_event_id {
            request = request.header("last-event-id", last);
        }
        let response = request.send().await.map_err(Error::Client)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(Error::ServerDoesNotSupportSse);
        }
        let response = response.error_for_status().map_err(Error::Client)?;
        if !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"))
        {
            return Err(Error::ServerDoesNotSupportSse);
        }
        Ok(events(response))
    }
}

fn bounded(
    response: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send {
    // 限制整次响应原始字节，JSON、分块传输和未终止的 SSE 事件均在解析前受约束。
    response.bytes_stream().scan(0_usize, |size, chunk| {
        let result = chunk.map_err(std::io::Error::other).and_then(|chunk| {
            *size = size.saturating_add(chunk.len());
            if *size > MAX_WIRE_BYTES {
                return Err(std::io::Error::other("MCP 响应超过 1 MiB"));
            }
            Ok(chunk)
        });
        std::future::ready(Some(result))
    })
}

fn events(response: reqwest::Response) -> BoxStream<'static, Result<Sse, sse_stream::Error>> {
    SseStream::from_bytes_stream(bounded(response)).boxed()
}
