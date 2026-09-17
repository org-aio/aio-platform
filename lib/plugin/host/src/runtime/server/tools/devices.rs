use super::{model::MarketplaceItem, storage};
use crate::{
    generated::worker::{controller::device, model::{SubmitTask, Task, Worker}},
    identity::SessionContext,
    runtime::{RuntimeResponse, server::{RuntimeState, http_error::RuntimeError, request_context::authenticate}},
};
use anyhow::{Result, ensure};
use axum::{Json, extract::{Path, State}, http::HeaderMap};
use az_tool::ToolManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inventory {
    pub tools: BTreeMap<String, InstalledTool>,
    pub packages: BTreeMap<String, String>,
    pub can_install: bool,
    pub error: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstalledTool {
    pub version: String,
    pub state: String,
}

#[derive(Serialize)]
pub(super) struct DeviceTool {
    #[serde(flatten)]
    device: Worker,
    installed: Option<InstalledTool>,
    checked_at: Option<i64>,
    fresh: bool,
    can_install: bool,
    supported: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallRequest {
    worker_id: String,
    version: String,
}

pub(super) async fn report(
    State(state): State<RuntimeState>, headers: HeaderMap, Json(inventory): Json<Inventory>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let device = device(&state, &headers).await?;
    validate_inventory(&inventory)?;
    let mut tx = state.store.pool.begin().await?;
    // 设备只能回报自己的记录；撤销配对后不能重新添加安装能力。
    let updated = sqlx::query("UPDATE worker_devices SET capabilities=(SELECT coalesce(jsonb_agg(value),'[]'::jsonb) FROM jsonb_array_elements(capabilities) WHERE value<>'\"tools.install\"'::jsonb) || CASE WHEN $2 THEN '[\"tools.install\"]'::jsonb ELSE '[]'::jsonb END WHERE id=$1 AND state='active'")
        .bind(&device.id).bind(inventory.can_install).execute(&mut *tx).await?;
    if updated.rows_affected() != 1 { return Err(RuntimeError::unauthorized("设备已撤销")); }
    sqlx::query("INSERT INTO worker_tool_inventory(device_id,inventory) VALUES($1,$2) ON CONFLICT(device_id) DO UPDATE SET inventory=EXCLUDED.inventory,updated_at=now()")
        .bind(&device.id).bind(serde_json::to_value(inventory)?).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(RuntimeResponse { data: () }))
}

fn validate_inventory(inventory: &Inventory) -> Result<()> {
    ensure!(inventory.tools.len() <= 1000 && inventory.packages.len() <= 1000, "安装记录过多");
    for item in inventory.tools.values() {
        ensure!(item.version.len() <= 100 && ["installed", "executed", "failed", "installing", "uninstalling"].contains(&item.state.as_str()), "安装记录无效");
    }
    ensure!(inventory.error.as_ref().is_none_or(|s| s.len() <= 1000), "检测错误信息过长");
    Ok(())
}

async fn inventories(state: &RuntimeState, session: &SessionContext) -> Result<BTreeMap<String, (Inventory, i64, bool)>> {
    let rows: Vec<(String, serde_json::Value, i64, bool)> = sqlx::query_as("SELECT i.device_id,i.inventory,(extract(epoch FROM i.updated_at)*1000)::bigint,i.updated_at>now()-interval '2 minutes' FROM worker_tool_inventory i JOIN worker_devices d ON d.id=i.device_id WHERE d.tenant_id=$1 AND d.user_id=$2 AND d.state='active'")
        .bind(&session.tenant_id).bind(&session.user_id).fetch_all(&state.store.pool).await?;
    rows.into_iter().map(|(id, value, at, fresh)| Ok((id, (serde_json::from_value(value)?, at, fresh)))).collect()
}

// npm 直接安装的工具没有 AIO 记录，通过市场中标准的全局安装包名对应。
fn installed(manifest: &ToolManifest, platform: &str, inventory: &Inventory) -> Option<InstalledTool> {
    if let Some(record) = inventory.tools.get(&manifest.id) {
        return Some(record.clone());
    }
    let platform = if platform == "darwin" { "macos" } else if platform == "win32" { "windows" } else { platform };
    let plan = manifest.platforms.get(platform)?;
    let command = plan.install.first()?;
    if command.program != "npm" || command.args.len() != 3 || command.args[0] != "install" || command.args[1] != "--global" {
        return None;
    }
    let (package, _) = command.args[2].rsplit_once('@')?;
    inventory.packages.get(package).map(|version| InstalledTool { version: version.clone(), state: "installed".into() })
}

async fn device_tools(state: &RuntimeState, session: &SessionContext, manifest: &ToolManifest) -> Result<Vec<DeviceTool>> {
    let inventories = inventories(state, session).await?;
    let workers = state.workers.list(session).await?;
    Ok(workers.into_iter().filter(|w| ["online", "offline"].contains(&w.status.as_str())).map(|device| {
        let snapshot = inventories.get(&device.id);
        let platform = match device.platform.as_str() { "darwin" => "macos", "win32" => "windows", other => other };
        DeviceTool {
            installed: snapshot.and_then(|(value, _, _)| installed(manifest, &device.platform, value)),
            checked_at: snapshot.map(|(_, at, _)| *at),
            fresh: snapshot.is_some_and(|(_, _, fresh)| *fresh) && device.status == "online",
            can_install: snapshot.is_some_and(|(value, _, fresh)| value.can_install && *fresh && value.error.is_none()) && device.status == "online",
            supported: manifest.platforms.contains_key(platform),
            error: snapshot.and_then(|(value, _, _)| value.error.clone()),
            device,
        }
    }).collect())
}

pub(super) async fn list(State(state): State<RuntimeState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<RuntimeResponse<Vec<DeviceTool>>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let manifest = storage::entries(&state.store.pool).await?.into_iter().find_map(|entry| match entry {
        MarketplaceItem::Cli(entry) if entry.cli.id == id => Some(entry.cli), _ => None,
    }).ok_or_else(|| RuntimeError::not_found("CLI 条目不存在"))?;
    Ok(Json(RuntimeResponse { data: device_tools(&state, &session, &manifest).await? }))
}

pub(super) async fn install(State(state): State<RuntimeState>, headers: HeaderMap, Path(id): Path<String>, Json(request): Json<InstallRequest>) -> Result<Json<RuntimeResponse<Task>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let manifest = storage::get(&state.store.pool, &id, &request.version).await?.ok_or_else(|| RuntimeError::not_found("CLI 条目不存在或已删除"))?;
    let devices = device_tools(&state, &session, &manifest).await?;
    let target = devices.iter().find(|d| d.device.id == request.worker_id).ok_or_else(|| RuntimeError::not_found("设备不存在"))?;
    if !target.can_install || !target.supported { return Err(RuntimeError::bad_request("设备离线、检测失败或本机助手需要升级")); }
    if target.installed.is_some() { return Err(RuntimeError::bad_request("设备已有安装记录，请先在设备上处理现有版本")); }
    let task = state.workers.enqueue(&session, SubmitTask {
        id: uuid::Uuid::new_v4().to_string(), worker_id: request.worker_id, capability: "tools.install".into(),
        input: serde_json::json!({"id": id, "version": request.version}),
    }).await?;
    Ok(Json(RuntimeResponse { data: task }))
}

pub(super) async fn mark_installed(state: &RuntimeState, session: &SessionContext, entries: &mut [MarketplaceItem]) -> Result<()> {
    let inventories = inventories(state, session).await?;
    let devices = state.workers.list(session).await?;
    for entry in entries {
        let MarketplaceItem::Cli(entry) = entry else { continue; };
        entry.installed = devices.iter().any(|device| inventories.get(&device.id).is_some_and(|(inventory, _, _)| installed(&entry.cli, &device.platform, inventory).is_some_and(|record| ["installed", "executed"].contains(&record.state.as_str()))));
    }
    Ok(())
}
