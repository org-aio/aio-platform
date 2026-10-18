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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Caller {
    pub tenant_id: String,
    pub user_id: String,
    pub context_id: String,
    /// 宿主在分发本次调用时下发的会话权限快照。
    ///
    /// 这是**当次请求**的能力集合，不是长期凭据：插件不得缓存后跨请求复用，也
    /// 不得把它写回响应或持久化。权限缺失即为无权，调用方不能自行填充。
    pub permissions: Vec<String>,
}

impl Caller {
    /// 判定本次调用是否被授予某权限；`*` 表示宿主明确授予的全局权限。
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions
            .iter()
            .any(|candidate| candidate == permission || candidate == "*")
    }
}

#[derive(Clone)]
struct Ingress {
    token: String,
    tenant: String,
}

/// 解析宿主下发的权限列表；缺失即为空集，格式错误直接拒绝而不是静默截断。
fn read_permissions(headers: &axum::http::HeaderMap) -> Result<Vec<String>, StatusCode> {
    let mut values = headers.get_all("x-aio-permissions").iter();
    let Some(value) = values.next() else {
        return Ok(Vec::new());
    };
    if values.next().is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let value = value.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut permissions = Vec::new();
    for item in value.split(',') {
        // 权限名与宿主授权一致：非空、短、无可打印控制字符，禁止重复。
        if item.is_empty() || item.len() > 128 || item.chars().any(char::is_control) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        if permissions.iter().any(|existing| existing == item) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        permissions.push(item.to_owned());
    }
    Ok(permissions)
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
    let permissions = read_permissions(headers)?;
    Ok(Caller {
        tenant_id: tenant.into(),
        user_id: user.into(),
        context_id: context.into(),
        permissions,
    })
}
