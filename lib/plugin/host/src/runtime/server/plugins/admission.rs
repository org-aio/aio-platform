//! 停止后拒绝新请求，已有处理先完成；响应流收到取消后释放活动引用。

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::{
    body::{Body, Bytes, HttpBody},
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body::{Frame, SizeHint};
use tokio::sync::OwnedRwLockReadGuard;

use super::RuntimeState;

pub(in crate::runtime::server) async fn admit(
    State(state): State<RuntimeState>,
    request: Request,
    next: Next,
) -> Response {
    let lifecycle = &state.extensions.lifecycle;
    if lifecycle.cancellation.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let permit = lifecycle.requests.clone().read_owned().await;
    if lifecycle.cancellation.is_cancelled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let response = next.run(request).await;
    response.map(|body| {
        Body::new(ActiveBody {
            body: Some(body),
            permit: Some(permit),
            cancelled: Box::pin(lifecycle.cancellation.clone().cancelled_owned()),
        })
    })
}

struct ActiveBody {
    body: Option<Body>,
    permit: Option<OwnedRwLockReadGuard<()>>,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl HttpBody for ActiveBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            self.body = None;
            self.permit = None;
            return Poll::Ready(None);
        }
        let Some(body) = &mut self.body else {
            return Poll::Ready(None);
        };
        let frame = Pin::new(body).poll_frame(cx);
        if matches!(frame, Poll::Ready(None) | Poll::Ready(Some(Err(_)))) {
            self.body = None;
            self.permit = None;
        }
        // 保留所有帧，尤其是 gRPC 的 trailers；不能转换为仅含数据的流。
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.body.as_ref().is_none_or(HttpBody::is_end_stream)
    }

    fn size_hint(&self) -> SizeHint {
        self.body
            .as_ref()
            .map(HttpBody::size_hint)
            .unwrap_or_default()
    }
}
