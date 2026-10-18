//! 受控身份服务：进程插件不能直连身份库，只能提交固定动作。
//!
//! 门控分两层：
//! 1. 版本在清单里声明对应能力：账号管理用 `identity_provider`，租户目录与
//!    租户写操作用 `tenant_provider`；
//! 2. 调用必须绑定宿主登记的**真实交互会话**，并按动作复核会话权限。
//!
//! 会话与会话权限都由宿主自己读取；插件提交的任何身份字段都不作数，因此请求
//! 里没有会话标识，也不存在“插件拿不到会话 ID 就无法调用”的自举问题。

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

/// 创建账号与重置密码属于账号管理。
const ACCOUNT_PERMISSION: &str = "rbac:manage";
/// 创建或重命名租户属于租户管理；列出租户和切换租户只需有效成员关系。
const TENANT_PERMISSION: &str = "tenant:manage";
/// 权限读写全部要求 `rbac:manage`。
const RBAC_PERMISSION: &str = "rbac:manage";

/// 把宿主租户视图转成契约形状；两者字段一致，转换只做字段搬运。
fn to_view(tenant: crate::identity::TenantView) -> az_plugin_contract::process::IdentityTenant {
    az_plugin_contract::process::IdentityTenant {
        id: tenant.id,
        label: tenant.label,
        current: tenant.current,
    }
}

/// 取受控权限提供者；未装配时任何 `rbac_provider` 调用一律拒绝。
fn access_control<'a>(
    gateway: &Gateway,
    components: &'a super::super::Components,
) -> Result<&'a Arc<dyn crate::identity::AccessControlProvider>> {
    ensure!(gateway.rbac_provider, "process 未获权限管理能力");
    components
        .access_control
        .as_ref()
        .context("宿主未装配权限管理提供者")
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

    // 只接受宿主自己登记的交互上下文；会话由宿主从登记表读取。
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
        grants: az_plugin_contract::CapabilityGrants::default(),
    };
    let (session, _) = components.services.management_session(&scope)?;

    match request.operation {
        IdentityOperation::CreateUser {
            account,
            display_name,
            password,
        } => {
            ensure!(gateway.identity_provider, "process 未获身份提供能力");
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
                access_control: None,
            })
        }
        IdentityOperation::ResetPassword {
            user_id,
            new_password,
        } => {
            ensure!(gateway.identity_provider, "process 未获身份提供能力");
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
                access_control: None,
            })
        }
        IdentityOperation::ListTenants => {
            ensure!(gateway.tenant_provider, "process 未获租户提供能力");
            // 只读且限定在调用者自己的成员关系内，无需额外权限位。
            let tenants = components.identity.list_tenants(&session).await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: tenants.into_iter().map(to_view).collect(),
                access_control: None,
            })
        }
        IdentityOperation::CreateTenant { label } => {
            ensure!(gateway.tenant_provider, "process 未获租户提供能力");
            ensure!(
                authorized(&session, TENANT_PERMISSION),
                "当前角色没有租户管理权限"
            );
            let tenant = components.identity.create_tenant(&session, &label).await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: vec![to_view(tenant)],
                access_control: None,
            })
        }
        IdentityOperation::RenameTenant { label } => {
            ensure!(gateway.tenant_provider, "process 未获租户提供能力");
            ensure!(
                authorized(&session, TENANT_PERMISSION),
                "当前角色没有租户管理权限"
            );
            let tenant = components
                .identity
                .rename_current_tenant(&session, &label)
                .await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: vec![to_view(tenant)],
                access_control: None,
            })
        }
        IdentityOperation::SwitchTenant { tenant_id } => {
            ensure!(gateway.tenant_provider, "process 未获租户提供能力");
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
                access_control: None,
            })
        }
        IdentityOperation::AccessControlView => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            let view = access.access_control_view(&session).await?;
            Ok(IdentityResponse {
                user_id: None,
                session: None,
                tenants: Vec::new(),
                access_control: Some(to_access_view(view)),
            })
        }
        IdentityOperation::SaveRole {
            role_id,
            permissions,
            create,
        } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access
                .save_role(&session, &role_id, &permissions, create)
                .await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::DeleteRole { role_id } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.delete_role(&session, &role_id).await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::UpdateMember {
            user_id,
            display_name,
        } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access
                .update_member(&session, &user_id, &display_name)
                .await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::SetMemberRoles { user_id, roles } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.set_member_roles(&session, &user_id, &roles).await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::RemoveMember { user_id } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.remove_member(&session, &user_id).await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::AuthorizeMemberPasswordReset { user_id } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            // 只做权限裁决；实际密码写入由插件随后调用 `ResetPassword` 完成。
            access
                .authorize_member_password_reset(&session, &user_id)
                .await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::SaveDepartment { department } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.save_department(&session, department).await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::DeleteDepartment { id } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.delete_department(&session, &id).await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::SavePost { post } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.save_post(&session, post).await?;
            Ok(IdentityResponse::empty())
        }
        IdentityOperation::DeletePost { id } => {
            let access = access_control(gateway, &components)?;
            ensure!(
                authorized(&session, RBAC_PERMISSION),
                "当前角色没有权限管理权限"
            );
            access.delete_post(&session, &id).await?;
            Ok(IdentityResponse::empty())
        }
    }
}

/// 权限视图在宿主与契约间共用同一形状，转换仅做类型搬运。
fn to_access_view(
    view: crate::identity::AccessControlView,
) -> az_plugin_contract::process::AccessControlView {
    az_plugin_contract::process::AccessControlView {
        users: view
            .users
            .into_iter()
            .map(|user| az_plugin_contract::process::AccessControlUser {
                id: user.id,
                account: user.account,
                display_name: user.display_name,
                roles: user.roles,
            })
            .collect(),
        roles: view
            .roles
            .into_iter()
            .map(|role| az_plugin_contract::process::AccessControlRole {
                id: role.id,
                permissions: role.permissions,
                member_count: role.member_count,
            })
            .collect(),
        departments: view.departments.into_iter().map(to_org).collect(),
        posts: view.posts.into_iter().map(to_org).collect(),
        current_user_id: view.current_user_id,
        grantable_permissions: view.grantable_permissions,
    }
}

fn to_org(
    item: az_plugin_contract::process::AccessControlOrganization,
) -> az_plugin_contract::process::AccessControlOrganization {
    item
}

#[cfg(test)]
mod tests {
    use super::{ACCOUNT_PERMISSION, TENANT_PERMISSION, authorized};

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
    fn tenant_writes_require_the_tenant_permission() {
        assert!(!authorized(&session(&[]), TENANT_PERMISSION));
        assert!(!authorized(
            &session(&[ACCOUNT_PERMISSION]),
            TENANT_PERMISSION
        ));
        assert!(authorized(&session(&["tenant:manage"]), TENANT_PERMISSION));
        assert!(authorized(&session(&["*"]), TENANT_PERMISSION));
    }

    #[test]
    fn account_writes_require_the_account_permission() {
        assert!(!authorized(&session(&[]), ACCOUNT_PERMISSION));
        assert!(!authorized(
            &session(&["tenant:manage"]),
            ACCOUNT_PERMISSION
        ));
        assert!(authorized(&session(&["rbac:manage"]), ACCOUNT_PERMISSION));
        assert!(authorized(&session(&["*"]), ACCOUNT_PERMISSION));
    }
}
