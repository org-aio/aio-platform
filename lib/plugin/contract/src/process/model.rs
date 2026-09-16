use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub abi_version: u32,
    pub tenant_id: String,
    pub database_url: Option<String>,
    pub encryption_key: Option<String>,
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
