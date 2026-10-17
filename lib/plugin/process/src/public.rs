use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
};
use az_plugin_contract::public::PublicRoute;
use subtle::ConstantTimeEq;

use super::Host;

/// 公开入口只有已绑定的租户，不携带用户、会话或可转授的交互上下文。
#[derive(Clone)]
pub struct PublicCaller {
    pub tenant_id: String,
}

#[derive(Clone)]
struct Ingress {
    token: String,
    tenant: String,
    routes: Vec<PublicRoute>,
}

impl Host {
    /// 仅用于清单明确声明的匿名 GET/HEAD 路由，仍须验证宿主令牌与租户。
    pub fn protect_public(&self, router: Router) -> Router {
        let ingress = Ingress {
            token: self.configuration.ingress_token.clone(),
            tenant: self.configuration.tenant_id.clone(),
            routes: self.configuration.public_routes.clone(),
        };
        router.layer(from_fn_with_state(ingress, authenticate))
    }
}

async fn authenticate(
    State(ingress): State<Ingress>,
    mut request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let single = |name: &str| {
        let mut values = headers.get_all(name).iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() {
            return None;
        }
        Some(value)
    };
    let valid = single("x-aio-token")
        .is_some_and(|v| v.as_bytes().ct_eq(ingress.token.as_bytes()).unwrap_u8() == 1)
        && single("x-aio-tenant-id") == Some(ingress.tenant.as_str())
        && single("x-aio-public") == Some("1")
        && ["x-aio-user-id", "x-aio-context", "cookie", "authorization"]
            .iter()
            .all(|name| !headers.contains_key(*name))
        && ingress
            .routes
            .iter()
            .any(|route| route.allows(request.method().as_str(), request.uri().path()));
    if !valid {
        return StatusCode::FORBIDDEN.into_response();
    }
    request.extensions_mut().insert(PublicCaller {
        tenant_id: ingress.tenant,
    });
    next.run(request).await
}
