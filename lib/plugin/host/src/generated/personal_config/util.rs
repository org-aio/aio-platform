use super::model::*;
use crate::runtime::server::http_error::RuntimeError;
use az_plugin_contract::{InvocationScope, RequestContext};
use base64::Engine;
use sha2::{Digest, Sha256};
use sqlx::Row;

pub(super) fn validate(request: &WriteEntry) -> Result<(), RuntimeError> {
    uuid::Uuid::parse_str(&request.id)?;
    if request.content.len() > 256 * 1024 || request.target.is_empty() || request.target.len() > 512
    {
        return Err(RuntimeError::bad_request("配置名称或正文超过限制"));
    }
    if request.deleted && !request.content.is_empty() {
        return Err(RuntimeError::bad_request("删除记录不能携带正文"));
    }
    if !matches!(request.layer.as_str(), "shared" | "os:darwin" | "os:linux") {
        let device = request
            .layer
            .strip_prefix("device:")
            .ok_or_else(|| RuntimeError::bad_request("配置层无效"))?;
        uuid::Uuid::parse_str(device)?;
    }
    match request.kind.as_str() {
        "file" => {
            if !valid_path(&request.target)
                || !matches!(
                    request.format.as_str(),
                    "text" | "jsonc" | "yjs-v1" | "yjs-blob-v1" | "yjs-folder-v1"
                )
            {
                return Err(RuntimeError::bad_request("配置路径或格式无效"));
            }
            // 宿主只加密存储更新，操作合并和正文校验在授权设备中执行。
            if request.format.starts_with("yjs-") && !request.deleted {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&request.content)
                    .map_err(|_| RuntimeError::bad_request("CRDT 更新必须使用 Base64"))?;
                if bytes.is_empty() || !request.secret {
                    return Err(RuntimeError::bad_request("CRDT 更新不能为空且必须加密保存"));
                }
            }
        }
        "env" => {
            if !valid_env(&request.target)
                || request.content.contains('\0')
                || request.target == "PATH"
            {
                return Err(RuntimeError::bad_request(
                    "环境变量名称无效；PATH 请使用 paths 项",
                ));
            }
        }
        "paths" => {
            if request.target != "PATH" {
                return Err(RuntimeError::bad_request("路径配置只支持 PATH"));
            }
            if !request.deleted {
                let paths: Vec<String> = serde_json::from_str(&request.content)?;
                if paths.len() > 64
                    || paths
                        .iter()
                        .any(|p| p.is_empty() || p.contains(['\0', '\n', ':']))
                {
                    return Err(RuntimeError::bad_request("附加路径列表无效"));
                }
            }
        }
        "function" => {
            if !valid_function(&request.target)
                || request.format != "bash"
                || !request.secret
                || request.executable
                || request.content.len() > 32 * 1024
                || request.content.contains('\0')
            {
                return Err(RuntimeError::bad_request("Bash 函数名称、正文或属性无效"));
            }
        }
        "command" => {
            if !request.target.as_bytes()[0].is_ascii_alphanumeric()
                || request.target.len() > 80
                || !request
                    .target
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_".contains(&c))
            {
                return Err(RuntimeError::bad_request("启动命令名称无效"));
            }
            if !request.deleted {
                let value: serde_json::Value = serde_json::from_str(&request.content)?;
                let bundle = value["darwin"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::bad_request("需要 macOS 应用标识"))?;
                if bundle.len() > 200
                    || !bundle.contains('.')
                    || !bundle
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
                {
                    return Err(RuntimeError::bad_request("应用标识无效"));
                }
            }
        }
        "asset" => {
            if request.target.contains(['/', '\\', '\0'])
                || request.target == "."
                || request.target == ".."
            {
                return Err(RuntimeError::bad_request("资源名称无效"));
            }
            if !request.deleted {
                let value: serde_json::Value = serde_json::from_str(&request.content)?;
                let snapshot = value["snapshotId"].as_str().unwrap_or("");
                let source = value["source"].as_str().unwrap_or("");
                if snapshot.len() != 64
                    || !snapshot.bytes().all(|b| b.is_ascii_hexdigit())
                    || !source.starts_with('/')
                    || source.contains('\0')
                    || source.split('/').any(|part| matches!(part, "." | ".."))
                {
                    return Err(RuntimeError::bad_request("归档资源引用无效"));
                }
            }
        }
        _ => return Err(RuntimeError::bad_request("配置类型无效")),
    }
    Ok(())
}
pub(super) fn valid_env(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
pub(super) fn valid_function(value: &str) -> bool {
    if matches!(value, "__proto__" | "constructor" | "prototype") {
        return false;
    }
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && value.len() <= 80
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub(super) fn valid_path(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && !path.chars().any(char::is_control)
        && !path.starts_with(".config/aio-space/")
        && path != ".config/aio-space"
        && path
            .split('/')
            .all(|p| !p.is_empty() && !matches!(p, "." | ".." | ".git" | "node_modules" | "target"))
}
pub(super) fn hash(request: &WriteEntry) -> Result<String, RuntimeError> {
    let value = serde_json::to_vec(&(
        &request.kind,
        &request.target,
        &request.layer,
        &request.format,
        request.secret,
        request.executable,
        request.deleted,
        &request.content,
    ))?;
    Ok(format!("{:x}", Sha256::digest(value)))
}
pub(super) fn scope(owner: &Owner) -> InvocationScope {
    InvocationScope {
        source_id: "aio-personal-config".into(),
        revision: String::new(),
        context: RequestContext {
            tenant_id: Some(owner.tenant.clone()),
            ..Default::default()
        },
        grants: Default::default(),
    }
}
pub(super) fn purpose(owner: &Owner, id: &str) -> String {
    format!("personal:{}:{}", owner.user, id)
}
pub(super) fn entry(row: &sqlx::postgres::PgRow) -> Result<Entry, RuntimeError> {
    Ok(Entry {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        target: row.try_get("target")?,
        layer: row.try_get("layer")?,
        format: row.try_get("format")?,
        secret: row.try_get("secret")?,
        executable: row.try_get("executable")?,
        deleted: row.try_get("deleted")?,
        revision: row.try_get("revision")?,
        hash: row.try_get("hash")?,
        size: row.try_get("size")?,
        updated_at: row.try_get("updated_ms")?,
    })
}
