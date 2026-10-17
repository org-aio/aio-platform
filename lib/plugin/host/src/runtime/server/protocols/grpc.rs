use std::time::{Duration, Instant};

use axum::{
    Router,
    body::Body,
    http::{Request as HttpRequest, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response as HttpResponse},
};
use az_plugin_contract::grpc::{
    capabilities_server::CapabilitiesServer, enhancements_server::EnhancementsServer,
};
use tonic::Status;

use super::{
    super::{RuntimeState, http_error::RuntimeError},
    grpc_capabilities::GrpcCapabilities,
    grpc_enhancements::GrpcEnhancements,
};

// 只存在于宿主请求扩展中，不能被脚本输入或协议载荷改写。
#[derive(Clone, Copy)]
pub(super) struct Deadline(pub Instant);

pub(in crate::runtime::server) fn router(
    state: RuntimeState,
) -> anyhow::Result<Router<RuntimeState>> {
    let native = CapabilitiesServer::new(GrpcCapabilities(state.clone()))
        .max_decoding_message_size(az_plugin_enhancement::MAX_VALUE_BYTES + 1024)
        // 目录最多 4096 条有界路径，Invoke 的 JSON 结果仍由公共调用链限制在 64 KiB。
        .max_encoding_message_size(1024 * 1024);
    let mut routes = tonic::service::Routes::new(native);
    if state
        .optional_service::<super::super::enhancements::Enhancements>()?
        .is_some()
    {
        let enhancements = EnhancementsServer::new(GrpcEnhancements(state))
            .max_decoding_message_size(az_plugin_enhancement::MAX_VALUE_BYTES + 1024)
            .max_encoding_message_size(az_plugin_enhancement::MAX_VALUE_BYTES + 1024);
        routes = routes.add_service(enhancements);
    }
    Ok(routes
        .into_axum_router()
        .reset_fallback()
        .route_layer(axum::middleware::from_fn(deadline))
        .with_state(()))
}

// Axum 合并路由不经过 Tonic 的监听器，在这里落实客户端截止时间和宿主等待上限。
async fn deadline(mut request: HttpRequest<Body>, next: Next) -> HttpResponse {
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
    let deadline = Instant::now() + duration;
    request.extensions_mut().insert(Deadline(deadline));
    if !duration.is_zero()
        && let Ok(response) = tokio::time::timeout_at(deadline.into(), next.run(request)).await
    {
        return response;
    }
    Status::deadline_exceeded("能力调用超过截止时间").into_http::<Body>()
}

pub(super) fn timeout(value: &str) -> Option<Duration> {
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

// 原生执行链和传输层可能在同一次轮询中到期，统一返回协议的截止时间状态。
pub(super) fn invocation_result<T>(
    result: Result<T, RuntimeError>,
    deadline: Option<Instant>,
) -> Result<T, Status> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(Status::deadline_exceeded("能力调用超过截止时间"));
    }
    result.map_err(status)
}

pub(super) fn status(error: RuntimeError) -> Status {
    match error.into_response().status() {
        StatusCode::UNAUTHORIZED => Status::unauthenticated("会话无效或已撤销"),
        StatusCode::FORBIDDEN => Status::permission_denied("当前身份没有该能力权限"),
        StatusCode::NOT_FOUND => Status::not_found("能力不可见或不存在"),
        StatusCode::SERVICE_UNAVAILABLE => Status::unavailable("能力服务暂不可用"),
        StatusCode::BAD_REQUEST => Status::failed_precondition("能力未就绪、输入无效或执行失败"),
        _ => Status::internal("能力服务调用失败"),
    }
}
