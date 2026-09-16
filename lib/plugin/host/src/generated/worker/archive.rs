use super::{
    controller::device,
    model::DeviceIdentity,
    util::{digest, secret},
};
use crate::runtime::{
    RuntimeResponse,
    server::{RuntimeState, http_error::RuntimeError},
};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use az_plugin_contract::{InvocationScope, RequestContext};
use serde_json::json;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
const PREFIX: &str = "/api/runtime/workers/archive/";
const TYPES: [&str; 5] = ["data", "keys", "locks", "snapshots", "index"];
const MAX_BYTES: usize = 32 * 1024 * 1024;

pub(super) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/workers/archive-config", get(configuration))
        .route(PREFIX, any(storage))
        .route("/api/runtime/workers/archive/{*path}", any(storage))
}
fn root(state: &RuntimeState, identity: &DeviceIdentity) -> PathBuf {
    let base = std::env::var_os("AIO_WORKER_STORAGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| state.config.cache_root.join("worker-archives"));
    base.join(digest(&format!(
        "{}:{}:{}",
        identity.tenant.len(),
        identity.tenant,
        identity.user
    )))
}
async fn configuration(
    State(state): State<RuntimeState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, RuntimeError> {
    let identity = device(&state, &headers).await?;
    allowed(&identity)?;
    let mut tx = state.store.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('worker-vault',0))")
        .execute(&mut *tx)
        .await?;
    let keyring = state.worker_keyring()?;
    let scope = InvocationScope {
        source_id: "aio-worker-archives".into(),
        revision: String::new(),
        context: RequestContext {
            tenant_id: Some(identity.tenant.clone()),
            ..Default::default()
        },
        grants: Default::default(),
    };
    let purpose = format!("archive:{}", identity.user);
    let existing: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT ciphertext FROM worker_vaults WHERE tenant_id=$1 AND user_id=$2",
    )
    .bind(&identity.tenant)
    .bind(&identity.user)
    .fetch_optional(&mut *tx)
    .await?;
    let password = if let Some(ciphertext) = existing {
        String::from_utf8(keyring.open(&scope, &purpose, &ciphertext)?)?
    } else {
        let password = secret()?;
        let ciphertext = keyring.seal(&scope, &purpose, password.as_bytes())?;
        sqlx::query("INSERT INTO worker_vaults(tenant_id,user_id,ciphertext) VALUES($1,$2,$3)")
            .bind(&identity.tenant)
            .bind(&identity.user)
            .bind(ciphertext)
            .execute(&mut *tx)
            .await?;
        password
    };
    tx.commit().await?;
    let initialized = tokio::fs::try_exists(root(&state, &identity).join("config")).await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(RuntimeResponse {
            data: json!({"password":password,"path":PREFIX,"initialized":initialized}),
        }),
    )
        .into_response())
}
fn allowed(identity: &DeviceIdentity) -> Result<(), RuntimeError> {
    if !identity
        .capabilities
        .iter()
        .any(|v| v.starts_with("space.archive"))
    {
        return Err(RuntimeError::forbidden("设备未授权归档能力"));
    }
    Ok(())
}
async fn storage(
    State(state): State<RuntimeState>,
    request: Request,
) -> Result<Response, RuntimeError> {
    let identity = device(&state, request.headers()).await?;
    allowed(&identity)?;
    let path = request
        .uri()
        .path()
        .strip_prefix(PREFIX)
        .context("归档路径无效")?
        .to_owned();
    let root = root(&state, &identity);
    let method = request.method().clone();
    if path.is_empty() {
        if method != Method::POST || request.uri().query() != Some("create=true") {
            return Ok(StatusCode::FORBIDDEN.into_response());
        }
        for kind in TYPES {
            tokio::fs::create_dir_all(root.join(kind)).await?;
        }
        return Ok(StatusCode::OK.into_response());
    }
    let parts: Vec<_> = path.trim_end_matches('/').split('/').collect();
    if parts.len() == 1 && TYPES.contains(&parts[0]) && method == Method::GET {
        let mut entries = Vec::new();
        let directory = root.join(parts[0]);
        if tokio::fs::try_exists(&directory).await? {
            let mut files = tokio::fs::read_dir(directory).await?;
            while let Some(file) = files.next_entry().await? {
                let name = file.file_name().to_string_lossy().into_owned();
                if valid_name(&name) {
                    entries.push(json!({"name":name,"size":file.metadata().await?.len()}));
                }
                check(entries.len() <= 100_000, "归档索引超过上限")?;
            }
        }
        entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        return Ok((
            [
                (header::CONTENT_TYPE, "application/vnd.x.restic.rest.v2"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            serde_json::to_vec(&entries)?,
        )
            .into_response());
    }
    check(
        path == "config" || (parts.len() == 2 && TYPES.contains(&parts[0]) && valid_name(parts[1])),
        "归档对象路径无效",
    )?;
    let file = root.join(&path);
    match method {
        Method::GET | Method::HEAD => {
            if !tokio::fs::try_exists(&file).await? {
                return Ok(StatusCode::NOT_FOUND.into_response());
            }
            let bytes = tokio::fs::read(&file).await?;
            let length = bytes.len();
            let mut builder = Response::builder()
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CACHE_CONTROL, "no-store")
                .header(header::ACCEPT_RANGES, "bytes");
            if method == Method::HEAD {
                return Ok(builder
                    .header(header::CONTENT_LENGTH, length)
                    .body(Body::empty())?);
            }
            if let Some(range) = request
                .headers()
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok())
            {
                let (start, end) = range_bounds(range, length)?;
                builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{length}"),
                );
                return Ok(builder.body(Body::from(bytes[start..=end].to_vec()))?);
            }
            Ok(builder.body(Body::from(bytes))?)
        }
        Method::POST => {
            let bytes = to_bytes(request.into_body(), MAX_BYTES).await?;
            check(!bytes.is_empty(), "归档对象不能为空")?;
            if path != "config" {
                check(digest_bytes(&bytes) == parts[1], "归档对象摘要不匹配")?;
            }
            tokio::fs::create_dir_all(file.parent().context("对象目录无效")?).await?;
            let temp = root.join(format!(".upload-{}", uuid::Uuid::new_v4()));
            let mut output = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .await?;
            output.write_all(&bytes).await?;
            output.sync_all().await?;
            drop(output);
            // 硬链接只在目标不存在时提交，避免并发初始化覆盖仓库配置。
            let committed = tokio::fs::hard_link(&temp, &file).await;
            tokio::fs::remove_file(&temp).await?;
            match committed {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    check(
                        tokio::fs::read(&file).await? == bytes,
                        "归档对象已存在且内容不同",
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
            Ok(StatusCode::OK.into_response())
        }
        Method::DELETE if parts.first() == Some(&"locks") => {
            match tokio::fs::remove_file(file).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            Ok(StatusCode::OK.into_response())
        }
        _ => Ok(StatusCode::FORBIDDEN.into_response()),
    }
}
fn valid_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn range_bounds(value: &str, length: usize) -> anyhow::Result<(usize, usize)> {
    let (start, end) = value
        .strip_prefix("bytes=")
        .and_then(|v| v.split_once('-'))
        .context("Range 无效")?;
    let start: usize = start.parse()?;
    let end = if end.is_empty() {
        length.saturating_sub(1)
    } else {
        end.parse::<usize>()?
    };
    ensure!(start <= end && end < length, "Range 超出范围");
    Ok((start, end))
}

fn check(condition: bool, message: &str) -> Result<(), RuntimeError> {
    if !condition {
        return Err(RuntimeError::bad_request(message));
    }
    Ok(())
}
