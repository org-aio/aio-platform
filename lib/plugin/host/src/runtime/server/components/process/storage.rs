use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use az_plugin_contract::storage::{MAX_OBJECT_BYTES, MAX_OBJECT_KEY_BYTES, OBJECT_TIMEOUT_SECONDS};

use super::{broker::active, model::Gateway};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObjectQuery {
    key: String,
}

enum Operation {
    Read,
    Write(Body),
    Remove,
}

pub(super) async fn read(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<ObjectQuery>,
    headers: HeaderMap,
) -> Response {
    invoke(&gateway, &headers, query.key, Operation::Read).await
}

pub(super) async fn write(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<ObjectQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    invoke(&gateway, &headers, query.key, Operation::Write(body)).await
}

pub(super) async fn remove(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<ObjectQuery>,
    headers: HeaderMap,
) -> Response {
    invoke(&gateway, &headers, query.key, Operation::Remove).await
}

async fn invoke(
    gateway: &Gateway,
    headers: &HeaderMap,
    key: String,
    operation: Operation,
) -> Response {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(OBJECT_TIMEOUT_SECONDS);
    let result = async {
        let permit = gateway
            .quota
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        if headers.get_all("x-aio-token").iter().count() != 1 {
            return Err(StatusCode::FORBIDDEN);
        }
        active(gateway, headers)
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
        let storage = gateway.storage.as_ref().ok_or(StatusCode::FORBIDDEN)?;
        if key.is_empty() || key.len() > MAX_OBJECT_KEY_BYTES {
            return Err(StatusCode::BAD_REQUEST);
        }
        // 先验证身份和并发，再读取有界二进制载荷；慢上传也占用同一请求预算。
        let removing = matches!(operation, Operation::Remove);
        let bytes = if let Operation::Write(body) = operation {
            Some(to_bytes(body, MAX_OBJECT_BYTES).await.map_err(|error| {
                use std::error::Error;
                if error
                    .source()
                    .is_some_and(|cause| cause.is::<http_body_util::LengthLimitError>())
                {
                    StatusCode::PAYLOAD_TOO_LARGE
                } else {
                    StatusCode::BAD_REQUEST
                }
            })?)
        } else {
            None
        };
        let _guard = gateway.storage_gate.lock().await;
        // 等待请求体或前序存储操作后，再检查安装状态。来源、租户与授权均来自启动快照。
        active(gateway, headers)
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
        if let Some(bytes) = bytes {
            storage.write(&key, &bytes).await.map_err(storage_error)?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        if removing {
            storage.remove(&key).await.map_err(storage_error)?;
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
        let bytes = storage.read(&key).await.map_err(storage_error)?;
        active(gateway, headers)
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
        Ok(download(bytes, permit, gateway.stopped.clone(), deadline))
    };
    let invocation = tokio::time::timeout_at(deadline, result);
    let outcome = tokio::select! {
        biased;
        _ = gateway.stopped.cancelled() => Err(StatusCode::FORBIDDEN),
        result = invocation => result.unwrap_or(Err(StatusCode::REQUEST_TIMEOUT)),
    };
    match outcome {
        Ok(response) => response,
        Err(status) => (status, "对象存储操作失败").into_response(),
    }
}

fn download(
    bytes: Vec<u8>,
    permit: tokio::sync::OwnedSemaphorePermit,
    stopped: tokio_util::sync::CancellationToken,
    deadline: tokio::time::Instant,
) -> Response {
    let length = bytes.len();
    // 响应传输完毕前仍占用额度，慢下载不能绕过上限堆积完整对象。
    let stream =
        futures_util::stream::unfold((Bytes::from(bytes), permit), move |(mut bytes, permit)| {
            let stopped = stopped.clone();
            async move {
                if bytes.is_empty() {
                    return None;
                }
                if stopped.is_cancelled() || tokio::time::Instant::now() >= deadline {
                    return Some((
                        Err(std::io::Error::other("对象下载已停止")),
                        (Bytes::new(), permit),
                    ));
                }
                let chunk = bytes.split_to(bytes.len().min(64 * 1024));
                Some((Ok(chunk), (bytes, permit)))
            }
        });
    (
        [
            ("content-type", "application/octet-stream".to_owned()),
            ("cache-control", "no-store".to_owned()),
            ("content-length", length.to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

fn storage_error(error: anyhow::Error) -> StatusCode {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        StatusCode::NOT_FOUND
    } else {
        // 不把后端文件路径、连接信息或其他租户内容返回给进程。
        eprintln!("Sidecar 对象存储后端失败: {error:#}");
        StatusCode::BAD_GATEWAY
    }
}
