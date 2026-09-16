use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 租户、用户和设备来自宿主身份，不接受客户端自行指定所有者。
#[cfg(feature = "server")]
#[derive(Clone)]
pub(crate) struct Owner {
    pub tenant: String,
    pub user: String,
    pub device: Option<String>,
}

/// 配置元数据可列出；正文通过单独读取接口按需解密。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub layer: String,
    pub format: String,
    pub secret: bool,
    pub executable: bool,
    pub deleted: bool,
    pub revision: i64,
    pub hash: String,
    pub size: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteEntry {
    pub id: String,
    pub expected: Option<i64>,
    pub kind: String,
    pub target: String,
    pub layer: String,
    pub format: String,
    pub secret: bool,
    pub executable: bool,
    pub deleted: bool,
    pub content: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Content {
    pub entry: Entry,
    pub content: String,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub revision: i64,
    pub entries: Vec<Entry>,
    pub devices: Vec<SyncDevice>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncDevice {
    pub id: String,
    pub label: String,
    pub platform: String,
    pub report: Value,
    pub resolutions: Value,
    pub updated_at: i64,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub device: String,
    pub entry: String,
    pub local: Option<String>,
    pub remote: String,
    pub side: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Changes {
    pub after: i64,
    pub wait: u8,
}
#[derive(Serialize, Deserialize)]
pub struct Access {
    pub enabled: bool,
}
