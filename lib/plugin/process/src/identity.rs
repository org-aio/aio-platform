//! 宿主身份服务客户端：插件不能直连身份库，只能提交固定动作。
//!
//! 调用只携带宿主登记的交互上下文；真实会话、租户与权限全部由宿主读取。
//! 客户端不接触会话标识，也不接受调用方自造的身份字段。

use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use az_plugin_contract::process::{
    AccessControlDepartment, AccessControlPost, AccessControlView, IdentityOperation,
    IdentityRequest, IdentityResponse, IdentitySession, IdentityTenant,
};
use reqwest::{Client, Url};

use super::Host;

/// broker 身份调用的一次性截止时间：身份操作涉及密码摘要，留出充足但有限的窗口。
const IDENTITY_TIMEOUT_SECONDS: u64 = 30;

/// 通过宿主 Unix broker 执行受控身份操作。
#[derive(Clone)]
pub struct IdentityService {
    client: Client,
}

impl Host {
    /// 创建身份客户端；插件须在清单声明身份或租户提供能力，权限仍由宿主逐次复核。
    pub fn identity(&self) -> Result<IdentityService> {
        ensure!(
            self.configuration.identity_provider
                || self.configuration.tenant_provider
                || self.configuration.rbac_provider,
            "宿主未授予身份、租户或权限提供能力"
        );
        let mut headers = reqwest::header::HeaderMap::new();
        let mut token = reqwest::header::HeaderValue::from_str(&self.configuration.ingress_token)?;
        token.set_sensitive(true);
        headers.insert("x-aio-token", token);
        let client = Client::builder()
            .unix_socket(self.configuration.broker_socket.as_str())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(IDENTITY_TIMEOUT_SECONDS))
            .build()?;
        Ok(IdentityService { client })
    }
}

impl IdentityService {
    /// 提交一次身份操作；`context_id` 是宿主登记的交互上下文，会话由宿主解析。
    async fn invoke(
        &self,
        context_id: &str,
        operation: IdentityOperation,
    ) -> Result<IdentityResponse> {
        ensure!(!context_id.is_empty(), "身份调用缺少交互上下文");
        let request = IdentityRequest { operation };
        let url = Url::parse("http://localhost/identity/invoke")?;
        let body = serde_json::to_vec(&request).context("序列化身份请求失败")?;
        let response = self
            .client
            .post(url)
            .header("x-aio-context", context_id)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .context("身份服务调用失败")?;
        let status = response.status();
        let bytes = response.bytes().await.context("读取身份响应失败")?;
        if !status.is_success() {
            // 保留宿主给出的原因，但不把凭据或整段响应写入日志。
            let message = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|detail| {
                    detail
                        .get("error")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "身份操作被宿主机拒绝".to_owned());
            anyhow::bail!("{message}");
        }
        serde_json::from_slice(&bytes).context("身份服务响应无效")
    }

    /// 在当前租户下创建账号，返回新用户 ID。
    pub async fn create_user(
        &self,
        context_id: &str,
        account: &str,
        display_name: &str,
        password: &str,
    ) -> Result<String> {
        let response = self
            .invoke(
                context_id,
                IdentityOperation::CreateUser {
                    account: account.to_owned(),
                    display_name: display_name.to_owned(),
                    password: password.to_owned(),
                },
            )
            .await?;
        response.user_id.context("身份服务未返回用户 ID")
    }

    /// 重置指定账号密码；租户归属由宿主复核。
    pub async fn reset_password(
        &self,
        context_id: &str,
        user_id: &str,
        new_password: &str,
    ) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::ResetPassword {
                user_id: user_id.to_owned(),
                new_password: new_password.to_owned(),
            },
        )
        .await?;
        Ok(())
    }

    /// 把当前会话切换到目标租户，返回切换后的会话。
    pub async fn switch_tenant(
        &self,
        context_id: &str,
        tenant_id: &str,
    ) -> Result<IdentitySession> {
        let response = self
            .invoke(
                context_id,
                IdentityOperation::SwitchTenant {
                    tenant_id: tenant_id.to_owned(),
                },
            )
            .await?;
        response.session.context("身份服务未返回新会话")
    }

    /// 列出当前用户加入的租户。
    pub async fn list_tenants(&self, context_id: &str) -> Result<Vec<IdentityTenant>> {
        let response = self
            .invoke(context_id, IdentityOperation::ListTenants)
            .await?;
        Ok(response.tenants)
    }

    /// 创建租户并返回它；宿主会把调用者设为管理员。
    pub async fn create_tenant(&self, context_id: &str, label: &str) -> Result<IdentityTenant> {
        let response = self
            .invoke(
                context_id,
                IdentityOperation::CreateTenant {
                    label: label.to_owned(),
                },
            )
            .await?;
        response
            .tenants
            .into_iter()
            .next()
            .context("身份服务未返回新租户")
    }

    /// 重命名当前租户并返回更新后的视图。
    pub async fn rename_current_tenant(
        &self,
        context_id: &str,
        label: &str,
    ) -> Result<IdentityTenant> {
        let response = self
            .invoke(
                context_id,
                IdentityOperation::RenameTenant {
                    label: label.to_owned(),
                },
            )
            .await?;
        response
            .tenants
            .into_iter()
            .next()
            .context("身份服务未返回租户")
    }
}

impl IdentityService {
    /// 读取当前租户的权限视图；只读，仍要求 `rbac:manage`。
    pub async fn access_control_view(&self, context_id: &str) -> Result<AccessControlView> {
        let response = self
            .invoke(context_id, IdentityOperation::AccessControlView)
            .await?;
        response.access_control.context("权限服务未返回视图")
    }

    /// 创建或更新角色。
    pub async fn save_role(
        &self,
        context_id: &str,
        role_id: &str,
        permissions: &[String],
        create: bool,
    ) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::SaveRole {
                role_id: role_id.to_owned(),
                permissions: permissions.to_vec(),
                create,
            },
        )
        .await?;
        Ok(())
    }

    /// 删除角色。
    pub async fn delete_role(&self, context_id: &str, role_id: &str) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::DeleteRole {
                role_id: role_id.to_owned(),
            },
        )
        .await?;
        Ok(())
    }

    /// 更新成员显示名称。
    pub async fn update_member(
        &self,
        context_id: &str,
        user_id: &str,
        display_name: &str,
    ) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::UpdateMember {
                user_id: user_id.to_owned(),
                display_name: display_name.to_owned(),
            },
        )
        .await?;
        Ok(())
    }

    /// 覆盖成员的角色分配。
    pub async fn set_member_roles(
        &self,
        context_id: &str,
        user_id: &str,
        roles: &[String],
    ) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::SetMemberRoles {
                user_id: user_id.to_owned(),
                roles: roles.to_vec(),
            },
        )
        .await?;
        Ok(())
    }

    /// 把成员移出当前租户。
    pub async fn remove_member(&self, context_id: &str, user_id: &str) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::RemoveMember {
                user_id: user_id.to_owned(),
            },
        )
        .await?;
        Ok(())
    }

    /// 裁决当前管理员是否有权重置该成员密码；实际写入走 `reset_password`。
    pub async fn authorize_member_password_reset(
        &self,
        context_id: &str,
        user_id: &str,
    ) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::AuthorizeMemberPasswordReset {
                user_id: user_id.to_owned(),
            },
        )
        .await?;
        Ok(())
    }

    /// 写入或更新部门。
    pub async fn save_department(
        &self,
        context_id: &str,
        department: AccessControlDepartment,
    ) -> Result<()> {
        self.invoke(context_id, IdentityOperation::SaveDepartment { department })
            .await?;
        Ok(())
    }

    /// 删除部门。
    pub async fn delete_department(&self, context_id: &str, id: &str) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::DeleteDepartment { id: id.to_owned() },
        )
        .await?;
        Ok(())
    }

    /// 写入或更新岗位。
    pub async fn save_post(&self, context_id: &str, post: AccessControlPost) -> Result<()> {
        self.invoke(context_id, IdentityOperation::SavePost { post })
            .await?;
        Ok(())
    }

    /// 删除岗位。
    pub async fn delete_post(&self, context_id: &str, id: &str) -> Result<()> {
        self.invoke(
            context_id,
            IdentityOperation::DeletePost { id: id.to_owned() },
        )
        .await?;
        Ok(())
    }
}
