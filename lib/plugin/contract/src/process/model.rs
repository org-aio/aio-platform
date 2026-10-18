use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub abi_version: u32,
    pub tenant_id: String,
    pub database_url: Option<String>,
    pub encryption_key: Option<String>,
    /// 当前进程是否持有对象存储授权；实际访问仍由 broker 实时复核。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub storage: bool,
    /// 当前版本是否被授予身份提供能力；每次调用仍由宿主的会话与权限复核。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub identity_provider: bool,
    /// 当前版本是否被授予租户目录/管理能力；每次调用仍按会话权限复核。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tenant_provider: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub public_routes: Vec<crate::public::PublicRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_base: Option<String>,
    pub ingress_token: String,
    pub broker_socket: String,
    pub endpoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_endpoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worker_capabilities: Vec<String>,
    pub services: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRequest {
    pub target: String,
    pub method: String,
    pub path: String,
    pub body: serde_json::Value,
    pub tenant_id: String,
    pub user_id: String,
    pub context_id: Option<String>,
    pub interactive: bool,
}

/// 宿主身份服务的受控操作。
///
/// 进程插件不能直接访问身份库，也不能提交任意 SQL。这里只暴露宿主明确实现的
/// 固定动作，每个动作都由宿主复核当前会话、租户与权限后执行。
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityRequest {
    /// 会话由宿主从交互上下文读取，插件不能提交或伪造会话标识。
    pub operation: IdentityOperation,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, tag = "kind")]
pub enum IdentityOperation {
    /// 在当前租户下创建账号；调用者必须持有宿主要求的权限。
    CreateUser {
        account: String,
        display_name: String,
        password: String,
    },
    /// 重置指定账号的密码；宿主自行复核目标与调用者的租户关系。
    ResetPassword {
        user_id: String,
        new_password: String,
    },
    /// 把当前会话切换到目标租户；宿主校验成员关系后返回新会话。
    SwitchTenant { tenant_id: String },
    /// 列出当前用户加入的租户；`current` 标记会话当前所在租户。
    ListTenants,
    /// 创建租户并把调用者设为管理员；需要宿主账号管理权限。
    CreateTenant { label: String },
    /// 重命名当前租户；需要宿主账号管理权限，且只能改会话所在租户。
    RenameTenant { label: String },
}

/// 身份操作结果；`session` 只在需要返回新会话的操作里出现。
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<IdentitySession>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tenants: Vec<IdentityTenant>,
}

/// 租户视图；字段与租户插件模型一致，便于插件直接消费。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityTenant {
    pub id: String,
    pub label: String,
    pub current: bool,
}

/// 宿主回传的会话视图；字段与宿主 `SessionContext` 一致，便于插件重建上下文。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentitySession {
    pub session_id: String,
    pub user_id: String,
    pub account: String,
    pub display_name: String,
    pub tenant_id: String,
    pub tenant_label: String,
    pub permissions: Vec<String>,
}

/// 进程插件通过宿主 broker 上报用量。价格与扣费规则由宿主决定。
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeterRequest {
    pub tenant_id: String,
    pub user_id: String,
    pub source_id: String,
    pub resource: String,
    pub quantity: i64,
    pub idempotency_key: String,
}
