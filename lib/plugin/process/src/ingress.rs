use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
};
use az_plugin_contract::process::Configuration;
use subtle::ConstantTimeEq;

/// 已通过宿主入口认证的调用者；Controller 通过 Extension 消费，不读取浏览器身份头。
#[derive(Clone)]
pub struct Caller {
    pub tenant_id: String,
    pub user_id: String,
    pub context_id: String,
}

#[derive(Clone)]
struct Ingress {
    token: String,
    tenant: String,
}

pub(super) fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub(super) fn protect(router: Router, configuration: &Configuration) -> Router {
    let ingress = Ingress {
        token: configuration.ingress_token.clone(),
        tenant: configuration.tenant_id.clone(),
    };
    router.layer(from_fn_with_state(ingress, authenticate))
}

async fn authenticate(
    State(ingress): State<Ingress>,
    mut request: Request,
    next: Next,
) -> Response {
    let caller = match read_caller(request.headers(), &ingress) {
        Ok(caller) => caller,
        Err(status) => return status.into_response(),
    };
    request.extensions_mut().insert(caller);
    next.run(request).await
}

fn read_caller(headers: &axum::http::HeaderMap, ingress: &Ingress) -> Result<Caller, StatusCode> {
    // 重复身份头同样拒绝，避免多个协议实现对首项/末项的解释不同。
    let read = |name: &str| {
        let mut values = headers.get_all(name).iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() {
            return None;
        }
        Some(value)
    };
    let token = read("x-aio-token").ok_or(StatusCode::UNAUTHORIZED)?;
    if token.as_bytes().ct_eq(ingress.token.as_bytes()).unwrap_u8() != 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let tenant = read("x-aio-tenant-id")
        .filter(|v| valid_identity(v))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user = read("x-aio-user-id")
        .filter(|v| valid_identity(v))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let context = read("x-aio-context")
        .filter(|v| valid_identity(v))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if tenant != ingress.tenant {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Caller {
        tenant_id: tenant.into(),
        user_id: user.into(),
        context_id: context.into(),
    })
}
