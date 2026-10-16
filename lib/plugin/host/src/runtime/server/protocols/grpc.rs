use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request as HttpRequest, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response as HttpResponse},
};
use az_plugin_contract::{
    EnhancementRequest,
    grpc::{
        InvokeRequest, InvokeResponse, ListRequest, ListResponse,
        enhancements_server::{Enhancements, EnhancementsServer},
    },
};
use tonic::{Request, Response, Status};

use super::{
    super::{RuntimeState, http_error::RuntimeError},
    access,
};

struct GrpcEnhancements(RuntimeState);

pub(in crate::runtime::server) fn router(state: RuntimeState) -> Router<RuntimeState> {
    let service = EnhancementsServer::new(GrpcEnhancements(state))
        .max_decoding_message_size(az_plugin_enhancement::MAX_VALUE_BYTES + 1024)
        .max_encoding_message_size(az_plugin_enhancement::MAX_VALUE_BYTES + 1024);
    tonic::service::Routes::new(service)
        .into_axum_router()
        .reset_fallback()
        .route_layer(axum::middleware::from_fn(deadline))
        .with_state(())
}

// Axum 合并路由不经过 Tonic 的监听器，在这里落实客户端截止时间和宿主等待上限。
async fn deadline(request: HttpRequest<Body>, next: Next) -> HttpResponse {
    let budget = request
        .headers()
        .get("grpc-timeout")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(timeout)
                .ok_or_else(|| Status::invalid_argument("grpc-timeout 无效"))
        })
        .transpose();
    let duration = match budget {
        Ok(budget) => budget
            .unwrap_or(Duration::from_secs(30))
            .min(Duration::from_secs(30)),
        Err(error) => return error.into_http::<Body>(),
    };
    if !duration.is_zero()
        && let Ok(response) = tokio::time::timeout(duration, next.run(request)).await
    {
        return response;
    }
    Status::deadline_exceeded("增强调用超过截止时间").into_http::<Body>()
}

fn timeout(value: &str) -> Option<Duration> {
    if !(2..=9).contains(&value.len()) || !value.is_ascii() {
        return None;
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    if !digits.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let amount: u64 = digits.parse().ok()?;
    match unit {
        "H" => Some(Duration::from_secs(amount * 3600)),
        "M" => Some(Duration::from_secs(amount * 60)),
        "S" => Some(Duration::from_secs(amount)),
        "m" => Some(Duration::from_millis(amount)),
        "u" => Some(Duration::from_micros(amount)),
        "n" => Some(Duration::from_nanos(amount)),
        _ => None,
    }
}

#[tonic::async_trait]
impl Enhancements for GrpcEnhancements {
    async fn list(&self, request: Request<ListRequest>) -> Result<Response<ListResponse>, Status> {
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let (revision, paths) = access::catalog(&self.0, &session).await.map_err(status)?;
        Ok(Response::new(ListResponse { revision, paths }))
    }

    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let headers = request.metadata().clone().into_headers();
        let session = access::session(&self.0, &headers).await.map_err(status)?;
        let request = request.into_inner();
        let input = serde_json::from_slice(&request.input_json)
            .map_err(|_| Status::invalid_argument("input_json 必须是完整 JSON 值"))?;
        let response = access::invoke(
            &self.0,
            &session,
            EnhancementRequest {
                path: request.path,
                input,
            },
        )
        .await
        .map_err(status)?;
        let output_json = serde_json::to_vec(&response.output)
            .map_err(|_| Status::internal("编码增强结果失败"))?;
        Ok(Response::new(InvokeResponse {
            revision: response.revision,
            output_json,
        }))
    }
}

fn status(error: RuntimeError) -> Status {
    match error.into_response().status() {
        StatusCode::UNAUTHORIZED => Status::unauthenticated("会话无效或已撤销"),
        StatusCode::FORBIDDEN => Status::permission_denied("当前身份没有该增强能力权限"),
        StatusCode::NOT_FOUND => Status::not_found("增强能力不存在"),
        StatusCode::SERVICE_UNAVAILABLE => Status::unavailable("增强服务暂不可用"),
        StatusCode::BAD_REQUEST => Status::failed_precondition("增强未激活、输入无效或执行失败"),
        _ => Status::internal("增强服务调用失败"),
    }
}
