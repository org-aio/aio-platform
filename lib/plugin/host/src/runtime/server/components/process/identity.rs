//! 受控身份服务：进程插件不能直连身份库，只能提交固定动作。
//!
//! 三重门控缺一不可：
//! 1. 该版本在清单里声明 `identity_provider` 能力；
//! 2. 调用绑定到宿主登记的**真实交互会话**（后台与跨插件派生调用被拒绝）；
//! 3. 会话持有该动作要求的宿主权限。
//!
//! 会话与会话权限都由宿主自己读取，插件提交的任何身份字段都不作数。

use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use az_plugin_contract::process::{
    IdentityOperation, IdentityRequest, IdentityResponse, IdentitySession,
};

use super::model::Gateway;

/// 创建账号与重置密码属于账号管理；切换租户只需要有效会话与成员关系。
const ACCOUNT_PERMISSION: &str = "rbac:manage";

/// 把宿主租户视图转成契约形状；两者字段一致，转换只做字段搬运。
fn to_view(tenant: crate::identity::TenantView) -> az_plugin_contract::process::IdentityTenant {
    az_plugin_contract::process::IdentityTenant {
        id: tenant.id,
        label: tenant.label,
        current: tenant.current,
    }
}

/// 权限判据：列出所需权限或宿主明确授予的通配权限即通过。
///
/// 端点与测试共用同一个函数，避免“测试通过但执行放宽”这类漂移。
fn authorized(session: &crate::identity::SessionContext, permission: &str) -> bool {
    session
        .permissions
        .iter()
        .any(|candidate| candidate == permission || candidate == "*")
}

pub(super) async fn invoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<IdentityRequest>,
) -> Response {
    match invoke_inner(&gateway, &headers, request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": format!("{error:#}")})),
        )
            .into_response(),
    }
}

async fn invoke_inner(
    gateway: &Gateway,
    headers: &HeaderMap,
    request: IdentityRequest,
) -> Result<IdentityResponse> {
    let _permit = gateway
        .quota
        .try_acquire()
        .context("process 调用并发已满")?;
    let components = super::broker::active(gateway, headers).await?;
    ensure!(gateway.identity_provider, "process 未获身份提供能力");

    // 只接受宿主自己登记的交互上下文；插件无法用伪造的会话标识绕过登记表。
    let id = headers
        .get("x-aio-context")
        .context("身份调用缺少交互上下文")?
        .to_str()?;
    let source = gateway.start.source.to_string();
    let context = components
        .services
        .interactive(id, &gateway.start.tenant, &source)?;
    let scope = az_plugin_contract::InvocationScope {
        source_id: source,
        revision: gateway.start.revision.clone(),
        context,
        grants: az_plugin_contract::CapabilityGrants {
            identity_provider: true,
            ..Default::default()
        },
    };
    // 会话来自宿主，插件提交的 sessionId 必须与它一致，不能借旧票据操作他人会话。
    let (session, _) = components.services.management_session(&scope)?;
    ensure!(
        session.session_id == request.session_id,
        "身份调用会话与宿主登记不一致"
    );

    match request.operation {
        IdentityOperation::CreateUser {
            account,
            display_name,
            password,
        } => {
            ensure!(
                authorized(&session, ACCOUNT_PERMISSION),
                "当前角色没有账号管理权限"
            );
            let user_id = components
                .identity
                .create_user(&session.tenant_id, &account, &display_name, &password)
                .await?;
            Ok(IdentityResponse {
                user_id: Some(user_id),
                session: None,
                tenants: Vec::new(),
            })
        }
        IdentityOperation::ResetPassword {
            user_id,
            new_password,
        } => {
            ensure!(
                authorized(&session, ACCOUNT_PERMISSION),
                "当前角色没有账号管理权限"
            );
            // 目标必须属于当前租户，避免跨租户重置他人在同类实例里的密码。
            ensure!(
                components
                    .identity
                    .member_active(&session.tenant_id, &user_id)
                    .await?,
                "目标账号不属于当前租户"
            );
            components
                .identity
                .reset_password(&user_id, &new_password)
                .await?;
            Ok(IdentityResponse {
                user_id: Some(user_id),
                session: None,
                tenants: Vec::new(),
            })
        }
        IdentityOperation::ListTenants => {
            // 只读且限定在调用者自己的成员关系内，无需额外权限位。
            let tenants = components.identity.list_tenants(&session).await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: tenants
                    .into_iter()
                    .map(|tenant| {
                        crate::runtime::server::components::process::identity::to_view(tenant)
                    })
                    .collect(),
            })
        }
        IdentityOperation::CreateTenant { label } => {
            ensure!(
                authorized(&session, ACCOUNT_PERMISSION),
                "当前角色没有账号管理权限"
            );
            let tenant = components.identity.create_tenant(&session, &label).await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: vec![to_view(tenant)],
            })
        }
        IdentityOperation::RenameTenant { label } => {
            ensure!(
                authorized(&session, ACCOUNT_PERMISSION),
                "当前角色没有账号管理权限"
            );
            let tenant = components
                .identity
                .rename_current_tenant(&session, &label)
                .await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: vec![to_view(tenant)],
            })
        }
        IdentityOperation::SwitchTenant { tenant_id } => {
            // 切换后仍保持同一用户：提供者校验成员关系，宿主无需额外权限位。
            let switched = components
                .identity
                .switch_tenant(&session, &tenant_id)
                .await?
                .context("当前账号不属于目标租户")?;
            Ok(IdentityResponse {
                user_id: None,
                session: Some(IdentitySession {
                    session_id: switched.session_id,
                    user_id: switched.user_id,
                    account: switched.account,
                    display_name: switched.display_name,
                    tenant_id: switched.tenant_id,
                    tenant_label: switched.tenant_label,
                    permissions: switched.permissions,
                }),
                tenants: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ACCOUNT_PERMISSION, authorized};

    fn session(permissions: &[&str]) -> crate::identity::SessionContext {
        crate::identity::SessionContext {
            session_id: "session".into(),
            user_id: "user".into(),
            account: "account".into(),
            display_name: "User".into(),
            tenant_id: "tenant".into(),
            tenant_label: "Tenant".into(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    #[test]
    fn tenant_writes_require_the_account_permission() {
        // 写操作与账号管理共用同一判据：没有该权限就不能创建或重命名租户。
        assert!(!authorized(
            &session(&["tenant:manage"]),
            ACCOUNT_PERMISSION
        ));
        assert!(authorized(
            &session(&[ACCOUNT_PERMISSION]),
            ACCOUNT_PERMISSION
        ));
    }

    #[test]
    fn account_permission_is_required_but_wildcard_is_honoured() {
        assert!(!authorized(&session(&[]), ACCOUNT_PERMISSION));
        assert!(!authorized(
            &session(&["tenant:manage"]),
            ACCOUNT_PERMISSION
        ));
        assert!(authorized(&session(&["rbac:manage"]), ACCOUNT_PERMISSION));
        assert!(authorized(&session(&["*"]), ACCOUNT_PERMISSION));
    }
}
