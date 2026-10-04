use super::model::*;
use crate::runtime::server::http_error::RuntimeError;
use az_plugin_contract::{InvocationScope, RequestContext};
use base64::Engine;
use sha2::{Digest, Sha256};
use sqlx::Row;

/// 单条内容上限；文本、图片与二进制共用同一配额。
pub(super) const MAX_CONTENT: usize = 8 * 1024 * 1024;
/// 每个分片密封后的上限受 Keyring 配额约束，这里保持较小以便流式读取。
pub(super) const CHUNK: usize = 64 * 1024;
/// 每个用户保留的最近条目数量，超出后按序号淘汰最旧记录。
pub(super) const MAX_ITEMS: i64 = 64;
const MAX_MIME: usize = 128;
const MAX_NAME: usize = 256;

pub(super) fn validate(request: &ClipWrite) -> Result<Vec<u8>, RuntimeError> {
    if !matches!(request.kind.as_str(), "text" | "image" | "binary") {
        return Err(RuntimeError::bad_request("剪切板条目类型无效"));
    }
    if request.mime.is_empty()
        || request.mime.len() > MAX_MIME
        || !request
            .mime
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"')
    {
        return Err(RuntimeError::bad_request("MIME 类型无效"));
    }
    if let Some(name) = &request.name
        && (name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control))
    {
        return Err(RuntimeError::bad_request("条目名称无效"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&request.data)
        .map_err(|_| RuntimeError::bad_request("正文必须为 Base64"))?;
    if bytes.len() > MAX_CONTENT {
        return Err(RuntimeError::bad_request("剪切板条目超过大小限制"));
    }
    if request.kind == "text" && std::str::from_utf8(&bytes).is_err() {
        return Err(RuntimeError::bad_request("文本条目必须是 UTF-8"));
    }
    Ok(bytes)
}

pub(super) fn chunks(plaintext: &[u8]) -> Vec<&[u8]> {
    if plaintext.is_empty() {
        return Vec::new();
    }
    plaintext.chunks(CHUNK).collect()
}

pub(super) fn hash(plaintext: &[u8]) -> String {
    format!("{:x}", Sha256::digest(plaintext))
}

pub(super) fn scope(owner: &Owner) -> InvocationScope {
    InvocationScope {
        source_id: "aio-clipboard".into(),
        revision: String::new(),
        context: RequestContext {
            tenant_id: Some(owner.tenant.clone()),
            ..Default::default()
        },
        grants: Default::default(),
    }
}

/// 每个分片的用途包含序号与下标，防止跨条目或跨分片重排密文。
pub(super) fn purpose(owner: &Owner, seq: i64, index: usize) -> String {
    format!("clipboard:{}:{}:{}", owner.user, seq, index)
}

pub(super) fn item(row: &sqlx::postgres::PgRow) -> Result<ClipItem, RuntimeError> {
    Ok(ClipItem {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        mime: row.try_get("mime")?,
        name: row.try_get("name")?,
        size: row.try_get("size")?,
        hash: row.try_get("hash")?,
        origin: row.try_get("origin_device")?,
        created_at: row.try_get("created_ms")?,
    })
}
