use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MountRequest {
    pub page_id: String,
}

#[derive(Serialize)]
pub(super) struct MountResponse {
    pub development: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abi: Option<u32>,
    pub token: String,
    pub src: String,
    pub revision: String,
    pub generation: String,
    pub session_context: String,
    pub context: String,
    pub assets: std::collections::BTreeMap<String, String>,
    pub asset_sizes: std::collections::BTreeMap<String, usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FrontendRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub body: String,
}
