use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContext {
    pub tenant_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub request_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityGrants {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enhancements: Vec<String>,
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
