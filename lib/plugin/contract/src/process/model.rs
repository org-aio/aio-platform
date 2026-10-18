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
    /// 当前版本是否被授予权限管理能力；每次调用仍按会话权限复核。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rbac_provider: bool,
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
    /// 读取当前租户的用户、角色、部门、岗位与可授予权限；只读。
    AccessControlView,
    /// 创建或更新角色；权限列表由宿主复核不得超出调用者自身授权。
    SaveRole {
        role_id: String,
        permissions: Vec<String>,
        create: bool,
    },
    /// 删除角色；仍被分配的角色由宿主拒绝删除。
    DeleteRole { role_id: String },
    /// 更新成员显示名称。
    UpdateMember {
        user_id: String,
        display_name: String,
    },
    /// 覆盖成员的角色分配。
    SetMemberRoles { user_id: String, roles: Vec<String> },
    /// 把成员移出当前租户；最后一个管理员与自身由宿主拒绝。
    RemoveMember { user_id: String },
    /// 校验当前管理员是否有权重置该成员密码；实际密码写入由宿主完成。
    AuthorizeMemberPasswordReset { user_id: String },
    /// 写入或更新部门。
    SaveDepartment { department: AccessControlDepartment },
    /// 删除部门；存在下级部门时由宿主拒绝。
    DeleteDepartment { id: String },
    /// 写入或更新岗位。
    SavePost { post: AccessControlPost },
    /// 删除岗位。
    DeletePost { id: String },
}

/// 部门入参；`id` 为空表示新建，由宿主生成稳定 ID。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessControlDepartment {
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: String,
    pub leader: String,
    pub phone: String,
    pub email: String,
    pub sort_order: i32,
    pub status: String,
}

/// 岗位入参；`id` 为空表示新建。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessControlPost {
    pub id: Option<String>,
    pub code: String,
    pub name: String,
    pub sort_order: i32,
    pub status: String,
    pub remark: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_control: Option<AccessControlView>,
}

/// 权限视图；字段与权限插件模型一致，便于插件直接渲染。
impl IdentityResponse {
    /// 无返回数据的确认响应。
    pub fn empty() -> Self {
        Self {
            user_id: None,
            session: None,
            tenants: Vec::new(),
            access_control: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessControlView {
    pub users: Vec<AccessControlUser>,
    pub roles: Vec<AccessControlRole>,
    pub departments: Vec<AccessControlOrganization>,
    pub posts: Vec<AccessControlOrganization>,
    pub current_user_id: String,
    pub grantable_permissions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessControlUser {
    pub id: String,
    pub account: String,
    pub display_name: String,
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessControlRole {
    pub id: String,
    pub permissions: Vec<String>,
    pub member_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessControlOrganization {
    pub id: String,
    pub code: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub leader: String,
    pub phone: String,
    pub email: String,
    pub sort_order: i32,
    pub status: String,
    pub remark: String,
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
