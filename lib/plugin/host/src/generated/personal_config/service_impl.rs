use super::{model::*, service::PersonalConfigService, util};
use crate::runtime::server::http_error::RuntimeError;
use az_plugin_runtime::Keyring;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

#[dill::component]
#[dill::interface(dyn PersonalConfigService)]
#[dill::scope(dill::Singleton)]
pub(crate) struct PersonalConfigServiceImpl {
    pool: PgPool,
    keyring: Keyring,
}

#[async_trait::async_trait]
impl PersonalConfigService for PersonalConfigServiceImpl {
    async fn revision(&self, owner: &Owner) -> Result<i64, RuntimeError> {
        Ok(sqlx::query_scalar(
            "SELECT revision FROM personal_config_heads WHERE tenant_id=$1 AND user_id=$2",
        )
        .bind(&owner.tenant)
        .bind(&owner.user)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }
    async fn catalog(&self, owner: &Owner) -> Result<Catalog, RuntimeError> {
        // 先读版本，再读条目；并发写入最多导致额外同步，不会跳过更新。
        let revision = self.revision(owner).await?;
        let rows=sqlx::query("SELECT *, (extract(epoch FROM updated_at)*1000)::bigint AS updated_ms FROM personal_config_entries WHERE tenant_id=$1 AND user_id=$2 ORDER BY kind,target,layer").bind(&owner.tenant).bind(&owner.user).fetch_all(&self.pool).await?;
        let entries = rows.iter().map(util::entry).collect::<Result<_, _>>()?;
        let rows=sqlx::query("SELECT d.*,w.label,w.platform,(extract(epoch FROM d.updated_at)*1000)::bigint AS updated_ms FROM personal_config_devices d JOIN worker_devices w ON w.id=d.device_id AND w.tenant_id=d.tenant_id AND w.user_id=d.user_id WHERE d.tenant_id=$1 AND d.user_id=$2 ORDER BY d.updated_at DESC").bind(&owner.tenant).bind(&owner.user).fetch_all(&self.pool).await?;
        let devices = rows
            .iter()
            .map(|r| {
                Ok(SyncDevice {
                    id: r.try_get("device_id")?,
                    label: r.try_get("label")?,
                    platform: r.try_get("platform")?,
                    report: r.try_get("report")?,
                    resolutions: r.try_get("resolutions")?,
                    updated_at: r.try_get("updated_ms")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?;
        Ok(Catalog {
            revision,
            entries,
            devices,
        })
    }
    async fn read(
        &self,
        owner: &Owner,
        id: &str,
        revision: Option<i64>,
    ) -> Result<Content, RuntimeError> {
        let (entry, ciphertext) = if let Some(revision) = revision {
            let row=sqlx::query("SELECT metadata,ciphertext FROM personal_config_history WHERE tenant_id=$1 AND user_id=$2 AND entry_id=$3 AND revision=$4").bind(&owner.tenant).bind(&owner.user).bind(id).bind(revision).fetch_optional(&self.pool).await?.ok_or_else(||RuntimeError::not_found("版本不存在"))?;
            (
                serde_json::from_value(row.try_get("metadata")?)?,
                row.try_get::<Vec<u8>, _>("ciphertext")?,
            )
        } else {
            let row=sqlx::query("SELECT *, (extract(epoch FROM updated_at)*1000)::bigint AS updated_ms FROM personal_config_entries WHERE tenant_id=$1 AND user_id=$2 AND id=$3").bind(&owner.tenant).bind(&owner.user).bind(id).fetch_optional(&self.pool).await?.ok_or_else(||RuntimeError::not_found("配置不存在"))?;
            (util::entry(&row)?, row.try_get("ciphertext")?)
        };
        let content = String::from_utf8(self.keyring.open(
            &util::scope(owner),
            &util::purpose(owner, id),
            &ciphertext,
        )?)?;
        Ok(Content { entry, content })
    }
    async fn write(&self, owner: &Owner, request: WriteEntry) -> Result<Entry, RuntimeError> {
        util::validate(&request)?;
        let hash = util::hash(&request)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO personal_config_heads(tenant_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(&owner.tenant).bind(&owner.user).execute(&mut *tx).await?;
        sqlx::query("SELECT revision FROM personal_config_heads WHERE tenant_id=$1 AND user_id=$2 FOR UPDATE").bind(&owner.tenant).bind(&owner.user).fetch_one(&mut *tx).await?;
        if let Some(device) = request.layer.strip_prefix("device:") {
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active')").bind(device).bind(&owner.tenant).bind(&owner.user).fetch_one(&mut *tx).await?;
            if !owned {
                return Err(RuntimeError::bad_request("覆盖层设备不存在或已撤销"));
            }
        }
        let old=sqlx::query("SELECT *, (extract(epoch FROM updated_at)*1000)::bigint AS updated_ms FROM personal_config_entries WHERE tenant_id=$1 AND user_id=$2 AND id=$3").bind(&owner.tenant).bind(&owner.user).bind(&request.id).fetch_optional(&mut *tx).await?;
        let previous = old.as_ref().map(util::entry).transpose()?;
        if let Some(entry) = &previous {
            if entry.format.starts_with("yjs-") && request.format != entry.format {
                return Err(RuntimeError::bad_request(
                    "CRDT 文件必须通过已升级的 Space 客户端编辑",
                ));
            }
            if entry.hash == hash {
                return Ok(entry.clone());
            }
            if entry.kind != request.kind
                || entry.target != request.target
                || entry.layer != request.layer
            {
                return Err(RuntimeError::bad_request("配置身份不能修改，请新建覆盖项"));
            }
        }
        if previous.as_ref().map(|e| e.revision) != request.expected {
            return Err(RuntimeError::conflict("配置已变化，请重新同步或处理冲突"));
        }
        let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM personal_config_entries WHERE tenant_id=$1 AND user_id=$2 AND kind=$3 AND target=$4 AND layer=$5 AND id<>$6)").bind(&owner.tenant).bind(&owner.user).bind(&request.kind).bind(&request.target).bind(&request.layer).bind(&request.id).fetch_one(&mut *tx).await?;
        if duplicate {
            return Err(RuntimeError::conflict("此作用域已经存在同名配置"));
        }
        let (size,count):(i64,i64)=sqlx::query_as("SELECT coalesce(sum(size),0)::bigint,count(*) FROM personal_config_entries WHERE tenant_id=$1 AND user_id=$2").bind(&owner.tenant).bind(&owner.user).fetch_one(&mut *tx).await?;
        if size - previous.as_ref().map_or(0, |e| e.size) + request.content.len() as i64
            > 32 * 1024 * 1024
            || (previous.is_none() && count >= 4096)
        {
            return Err(RuntimeError::bad_request("个人配置库达到容量限制"));
        }
        if let (Some(row), Some(entry)) = (&old, &previous) {
            sqlx::query("INSERT INTO personal_config_history(tenant_id,user_id,entry_id,revision,metadata,ciphertext) VALUES($1,$2,$3,$4,$5,$6)").bind(&owner.tenant).bind(&owner.user).bind(&request.id).bind(entry.revision).bind(serde_json::to_value(entry)?).bind(row.try_get::<Vec<u8>,_>("ciphertext")?).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM personal_config_history WHERE tenant_id=$1 AND user_id=$2 AND entry_id=$3 AND revision NOT IN (SELECT revision FROM personal_config_history WHERE tenant_id=$1 AND user_id=$2 AND entry_id=$3 ORDER BY revision DESC LIMIT 10)").bind(&owner.tenant).bind(&owner.user).bind(&request.id).execute(&mut *tx).await?;
        }
        let revision:i64=sqlx::query_scalar("UPDATE personal_config_heads SET revision=revision+1 WHERE tenant_id=$1 AND user_id=$2 RETURNING revision").bind(&owner.tenant).bind(&owner.user).fetch_one(&mut *tx).await?;
        let ciphertext = self.keyring.seal(
            &util::scope(owner),
            &util::purpose(owner, &request.id),
            request.content.as_bytes(),
        )?;
        let row=sqlx::query("INSERT INTO personal_config_entries(tenant_id,user_id,id,kind,target,layer,format,secret,executable,deleted,revision,hash,size,ciphertext) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) ON CONFLICT(tenant_id,user_id,id) DO UPDATE SET format=$7,secret=$8,executable=$9,deleted=$10,revision=$11,hash=$12,size=$13,ciphertext=$14,updated_at=now() RETURNING *, (extract(epoch FROM updated_at)*1000)::bigint AS updated_ms")
            .bind(&owner.tenant).bind(&owner.user).bind(&request.id).bind(&request.kind).bind(&request.target).bind(&request.layer).bind(&request.format).bind(request.secret).bind(request.executable).bind(request.deleted).bind(revision).bind(hash).bind(request.content.len() as i64).bind(ciphertext).fetch_one(&mut *tx).await?;
        let entry = util::entry(&row)?;
        tx.commit().await?;
        Ok(entry)
    }
    async fn history(&self, owner: &Owner, id: &str) -> Result<Vec<Entry>, RuntimeError> {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT metadata FROM personal_config_history WHERE tenant_id=$1 AND user_id=$2 AND entry_id=$3 ORDER BY revision DESC").bind(&owner.tenant).bind(&owner.user).bind(id).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()?)
    }
    async fn report(&self, owner: &Owner, report: Value) -> Result<(), RuntimeError> {
        let device = owner
            .device
            .as_ref()
            .ok_or_else(|| RuntimeError::forbidden("需要设备身份"))?;
        if !report.is_object() || serde_json::to_vec(&report)?.len() > 256 * 1024 {
            return Err(RuntimeError::bad_request("同步报告无效"));
        }
        sqlx::query("INSERT INTO personal_config_devices(tenant_id,user_id,device_id,report) VALUES($1,$2,$3,$4) ON CONFLICT(tenant_id,user_id,device_id) DO UPDATE SET report=$4,updated_at=now(),resolutions=CASE WHEN $4->>'phase'='complete' THEN '{}'::jsonb ELSE personal_config_devices.resolutions END").bind(&owner.tenant).bind(&owner.user).bind(device).bind(report).execute(&self.pool).await?;
        Ok(())
    }
    async fn resolve(&self, owner: &Owner, request: Resolution) -> Result<(), RuntimeError> {
        if owner.device.is_some() || !matches!(request.side.as_str(), "local" | "remote") {
            return Err(RuntimeError::forbidden("需要网页账号处理冲突"));
        }
        let mut tx = self.pool.begin().await?;
        let report:Value=sqlx::query_scalar("SELECT report FROM personal_config_devices WHERE tenant_id=$1 AND user_id=$2 AND device_id=$3 FOR UPDATE").bind(&owner.tenant).bind(&owner.user).bind(&request.device).fetch_optional(&mut *tx).await?.ok_or_else(||RuntimeError::not_found("设备同步记录不存在"))?;
        let conflict = report["conflicts"]
            .as_array()
            .and_then(|items| items.iter().find(|e| e["id"] == request.entry))
            .ok_or_else(|| RuntimeError::conflict("冲突已变化"))?;
        if conflict["local"] != json!(request.local) || conflict["remote"] != request.remote {
            return Err(RuntimeError::conflict("冲突版本已变化"));
        }
        sqlx::query("UPDATE personal_config_devices SET resolutions=jsonb_set(resolutions,ARRAY[$4]::text[],$5,true) WHERE tenant_id=$1 AND user_id=$2 AND device_id=$3").bind(&owner.tenant).bind(&owner.user).bind(&request.device).bind(&request.entry).bind(serde_json::to_value(&request)?).execute(&mut *tx).await?;
        sqlx::query("UPDATE personal_config_heads SET revision=revision+1 WHERE tenant_id=$1 AND user_id=$2").bind(&owner.tenant).bind(&owner.user).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    async fn access(&self, owner: &Owner, device: &str, enabled: bool) -> Result<(), RuntimeError> {
        if enabled && owner.device.as_deref() != Some(device) {
            return Err(RuntimeError::forbidden("请在目标设备本机启用配置同步"));
        }
        let count=sqlx::query("UPDATE worker_devices SET capabilities=CASE WHEN $4 THEN (capabilities-'config.sync') || '[\"config.sync\"]'::jsonb ELSE capabilities-'config.sync' END WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active'").bind(device).bind(&owner.tenant).bind(&owner.user).bind(enabled).execute(&self.pool).await?.rows_affected();
        if count != 1 {
            return Err(RuntimeError::not_found("设备不存在或已撤销"));
        }
        Ok(())
    }
}
