//! 宿主身份服务客户端：插件不能直连身份库，只能提交固定动作。
//!
//! 调用必须携带当前登录会话的标识；宿主会用它核对真实交互会话、租户与权限。
//! 客户端不缓存会话，也不接受调用方自造的身份字段。

use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use az_plugin_contract::process::{
    IdentityOperation, IdentityRequest, IdentityResponse, IdentitySession,
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
    /// 创建身份客户端；插件须在清单声明 `identity_provider`，权限仍由宿主逐次复核。
    pub fn identity(&self) -> Result<IdentityService> {
        ensure!(
            self.configuration.identity_provider,
            "宿主未授予身份提供能力"
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
    /// 提交一次身份操作；`context_id` 是宿主登记的交互上下文，不是会话标识本身。
    async fn invoke(
        &self,
        context_id: &str,
        session_id: &str,
        operation: IdentityOperation,
    ) -> Result<IdentityResponse> {
        ensure!(!context_id.is_empty(), "身份调用缺少交互上下文");
        ensure!(!session_id.is_empty(), "身份调用缺少会话");
        let request = IdentityRequest {
            session_id: session_id.to_owned(),
            operation,
        };
        let url = Url::parse("http://localhost/identity/invoke")?;
        let response = self
            .client
            .post(url)
            .header("x-aio-context", context_id)
            .json(&request)
            .send()
            .await
            .context("身份服务调用失败")?;
        let status = response.status();
        if !status.is_success() {
            // 保留宿主给出的原因，但不把凭据或整段响应写入日志。
            let detail: serde_json::Value = response.json().await.unwrap_or_default();
            let message = detail
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("身份操作被宿主机拒绝");
            anyhow::bail!("{message}");
        }
        response.json().await.context("身份服务响应无效")
    }

    /// 在当前租户下创建账号，返回新用户 ID。
    pub async fn create_user(
        &self,
        context_id: &str,
        session_id: &str,
        account: &str,
        display_name: &str,
        password: &str,
    ) -> Result<String> {
        let response = self
            .invoke(
                context_id,
                session_id,
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
        session_id: &str,
        user_id: &str,
        new_password: &str,
    ) -> Result<()> {
        self.invoke(
            context_id,
            session_id,
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
        session_id: &str,
        tenant_id: &str,
    ) -> Result<IdentitySession> {
        let response = self
            .invoke(
                context_id,
                session_id,
                IdentityOperation::SwitchTenant {
                    tenant_id: tenant_id.to_owned(),
                },
            )
            .await?;
        response.session.context("身份服务未返回新会话")
    }
}
