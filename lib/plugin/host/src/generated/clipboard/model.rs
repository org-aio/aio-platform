use serde::{Deserialize, Serialize};

/// 租户、用户和设备来自宿主身份，不接受客户端自行指定所有者。
#[cfg(feature = "server")]
#[derive(Clone)]
pub(crate) struct Owner {
    pub tenant: String,
    pub user: String,
    pub device: Option<String>,
}

/// 剪切板条目元数据可列出；正文通过单独读取接口按需解密。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipItem {
    pub id: String,
    /// text | image | binary，用于界面区分和本机写入策略。
    pub kind: String,
    pub mime: String,
    pub name: Option<String>,
    pub size: i64,
    pub hash: String,
    /// 产生该条目的设备标识；网页上传为空。
    pub origin: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipContent {
    pub item: ClipItem,
    /// Base64 编码的原始字节，文本与二进制使用同一通道。
    pub data: String,
}

/// 设备或网页提交的新条目；正文为 Base64，长度在宿主再次校验。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipWrite {
    pub kind: String,
    pub mime: String,
    #[serde(default)]
    pub name: Option<String>,
    pub data: String,
}

/// 分页按序号倒序返回，`cursor` 为继续翻页的位置。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipPage {
    pub cursor: i64,
    pub items: Vec<ClipItem>,
}

/// 当前剪切板槽位；`revision` 为最新条目的序号，0 表示为空。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipHead {
    pub revision: i64,
    pub item: Option<ClipItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipboardDevice {
    pub id: String,
    pub label: String,
    pub platform: String,
    pub enabled: bool,
    pub last_seen: Option<i64>,
}

/// 变更长轮询参数；`after` 为客户端已确认的版本。
#[derive(Deserialize)]
pub struct Changes {
    pub after: i64,
    pub wait: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Access {
    pub enabled: bool,
}

/// 显式投递；不指定设备时投递到全部已开通剪切板通道的设备。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Push {
    #[serde(default)]
    pub devices: Vec<String>,
}

/// 设备领取到的投递任务，携带条目元数据与正文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PushTask {
    pub id: String,
    pub content: ClipContent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushResult {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}
