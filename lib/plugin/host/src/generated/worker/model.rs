use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 设备主动声明可执行能力，路径授权由设备本机再次检查。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PairRequest {
    pub label: String,
    pub platform: String,
    pub capabilities: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pairing {
    pub device_id: String,
    pub code: String,
    pub token: String,
    pub expires_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Worker {
    pub id: String,
    pub label: String,
    pub platform: String,
    pub capabilities: Vec<String>,
    pub status: String,
    pub last_seen: Option<i64>,
}
/// 任务 ID 用于去重，租约只在领取后交给对应设备。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub worker_id: String,
    pub capability: String,
    pub input: Value,
    pub state: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub lease: Option<String>,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitTask {
    pub id: String,
    pub worker_id: String,
    pub capability: String,
    pub input: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompleteTask {
    pub lease: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub lease: String,
}

/// 同一次领取重试复用请求 ID，避免响应丢失后遗失任务租约。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
    pub request_id: String,
    pub wait_seconds: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesktopAccess {
    pub enabled: bool,
}
/// 本地 CLI 使用设备凭据开关工作区执行，不允许指定其他设备或能力。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAccess {
    pub enabled: bool,
}
#[derive(Clone)]
pub struct DeviceIdentity {
    pub id: String,
    pub tenant: String,
    pub user: String,
    pub capabilities: Vec<String>,
}
