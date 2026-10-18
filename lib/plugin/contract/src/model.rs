use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContext {
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub request_id: String,
    /// 调用者在本会话内被授予的权限。
    ///
    /// 由宿主在分发前写入，插件不能自行填充或提升。字段带默认值以保持 ABI 兼容，
    /// 但空白权限不会让插件获得任何能力：消费方必须以“包含所需权限”为判据。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityGrants {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enhancements: Vec<String>,
    /// 增强脚本可调用的宿主能力；必须同时在增强源码包中声明。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
    pub cryptography: bool,
    pub database: bool,
    pub storage: bool,
    pub management: bool,
    pub identity_provider: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationScope {
    pub source_id: String,
    pub revision: String,
    pub context: RequestContext,
    pub grants: CapabilityGrants,
}
