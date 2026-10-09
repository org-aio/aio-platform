use super::{model::*, service::ClipboardService, util};
use crate::runtime::server::http_error::RuntimeError;
use az_plugin_runtime::Keyring;
use base64::Engine;
use sqlx::{PgPool, Row};

#[dill::component]
#[dill::interface(dyn ClipboardService)]
#[dill::scope(dill::Singleton)]
pub(crate) struct ClipboardServiceImpl {
    pool: PgPool,
    keyring: Keyring,
}

#[async_trait::async_trait]
impl ClipboardService for ClipboardServiceImpl {
    async fn head(&self, owner: &Owner) -> Result<ClipHead, RuntimeError> {
        let revision = self.revision(owner).await?;
        let item = if revision == 0 {
            None
        } else {
            let row=sqlx::query("SELECT *,(extract(epoch FROM created_at)*1000)::bigint AS created_ms FROM clipboard_items WHERE tenant_id=$1 AND user_id=$2 AND seq=$3").bind(&owner.tenant).bind(&owner.user).bind(revision).fetch_optional(&self.pool).await?;
            row.as_ref().map(util::item).transpose()?
        };
        Ok(ClipHead { revision, item })
    }
    async fn list(
        &self,
        owner: &Owner,
        cursor: Option<i64>,
        limit: i64,
    ) -> Result<ClipPage, RuntimeError> {
        if !(1..=100).contains(&limit) {
            return Err(RuntimeError::bad_request("分页数量无效"));
        }
        let before = cursor.unwrap_or(i64::MAX);
        let rows=sqlx::query("SELECT *,(extract(epoch FROM created_at)*1000)::bigint AS created_ms FROM clipboard_items WHERE tenant_id=$1 AND user_id=$2 AND seq<$3 ORDER BY seq DESC LIMIT $4").bind(&owner.tenant).bind(&owner.user).bind(before).bind(limit + 1).fetch_all(&self.pool).await?;
        let page = rows
            .iter()
            .take(limit as usize)
            .map(|row| Ok((row.try_get::<i64, _>("seq")?, util::item(row)?)))
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        // 还有更多时以本页最后一条的序号作为下一页游标，否则归零。
        let next = if rows.len() > limit as usize {
            page.last().map(|(seq, _)| *seq).unwrap_or(0)
        } else {
            0
        };
        Ok(ClipPage {
            cursor: next,
            items: page.into_iter().map(|(_, item)| item).collect(),
        })
    }
    async fn read(&self, owner: &Owner, id: &str) -> Result<ClipContent, RuntimeError> {
        uuid::Uuid::parse_str(id)?;
        let row=sqlx::query("SELECT *,(extract(epoch FROM created_at)*1000)::bigint AS created_ms FROM clipboard_items WHERE tenant_id=$1 AND user_id=$2 AND id=$3").bind(&owner.tenant).bind(&owner.user).bind(id).fetch_optional(&self.pool).await?.ok_or_else(||RuntimeError::not_found("剪切板条目不存在"))?;
        let item = util::item(&row)?;
        let seq: i64 = row.try_get("seq")?;
        let data = self.assemble(owner, seq).await?;
        if util::hash(&data) != item.hash {
            return Err(RuntimeError::conflict("剪切板条目校验失败"));
        }
        Ok(ClipContent {
            item,
            data: base64::engine::general_purpose::STANDARD.encode(data),
        })
    }
    async fn write(&self, owner: &Owner, request: ClipWrite) -> Result<ClipItem, RuntimeError> {
        let bytes = util::validate(&request)?;
        let hash = util::hash(&bytes);
        let scope = util::scope(owner);
        let id = uuid::Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO clipboard_heads(tenant_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(&owner.tenant)
        .bind(&owner.user)
        .execute(&mut *tx)
        .await?;
        let seq:i64=sqlx::query_scalar("UPDATE clipboard_heads SET revision=revision+1 WHERE tenant_id=$1 AND user_id=$2 RETURNING revision").bind(&owner.tenant).bind(&owner.user).fetch_one(&mut *tx).await?;
        for (index, chunk) in util::chunks(&bytes).into_iter().enumerate() {
            let ciphertext = self
                .keyring
                .seal(&scope, &util::purpose(owner, seq, index), chunk)?;
            sqlx::query("INSERT INTO clipboard_chunks(tenant_id,user_id,seq,idx,ciphertext) VALUES($1,$2,$3,$4,$5)").bind(&owner.tenant).bind(&owner.user).bind(seq).bind(index as i32).bind(ciphertext).execute(&mut *tx).await?;
        }
        let row=sqlx::query("INSERT INTO clipboard_items(tenant_id,user_id,seq,id,kind,mime,name,size,hash,origin_device) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *,(extract(epoch FROM created_at)*1000)::bigint AS created_ms").bind(&owner.tenant).bind(&owner.user).bind(seq).bind(&id).bind(&request.kind).bind(&request.mime).bind(&request.name).bind(bytes.len() as i64).bind(&hash).bind(&owner.device).fetch_one(&mut *tx).await?;
        // 只保留最近若干条；先用同一阈值清理分片，再删除条目，避免无限增长或残留孤儿分片。
        let threshold = seq - util::MAX_ITEMS + 1;
        sqlx::query("DELETE FROM clipboard_chunks WHERE tenant_id=$1 AND user_id=$2 AND seq<$3")
            .bind(&owner.tenant)
            .bind(&owner.user)
            .bind(threshold)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM clipboard_items WHERE tenant_id=$1 AND user_id=$2 AND seq<$3")
            .bind(&owner.tenant)
            .bind(&owner.user)
            .bind(threshold)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        util::item(&row)
    }
    async fn devices(&self, owner: &Owner) -> Result<Vec<ClipboardDevice>, RuntimeError> {
        let rows=sqlx::query("SELECT id,label,note,platform,capabilities,last_seen,(extract(epoch FROM last_seen)*1000)::bigint AS last_ms FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND state='active' ORDER BY created_at DESC LIMIT 100").bind(&owner.tenant).bind(&owner.user).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                let capabilities: Vec<String> =
                    serde_json::from_value(row.try_get("capabilities")?)?;
                Ok(ClipboardDevice {
                    id: row.try_get("id")?,
                    label: row.try_get("label")?,
                    note: row.try_get("note")?,
                    platform: row.try_get("platform")?,
                    enabled: capabilities.iter().any(|c| c == "clipboard.sync"),
                    last_seen: row.try_get("last_ms")?,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()
    }
    async fn access(&self, owner: &Owner, device: &str, enabled: bool) -> Result<(), RuntimeError> {
        if enabled && owner.device.as_deref() != Some(device) {
            return Err(RuntimeError::forbidden("请在目标设备本机启用剪切板接力"));
        }
        let count=sqlx::query("UPDATE worker_devices SET capabilities=CASE WHEN $4 THEN (capabilities-'clipboard.sync') || '[\"clipboard.sync\"]'::jsonb ELSE capabilities-'clipboard.sync' END WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active'").bind(device).bind(&owner.tenant).bind(&owner.user).bind(enabled).execute(&self.pool).await?.rows_affected();
        if count != 1 {
            return Err(RuntimeError::not_found("设备不存在或已撤销"));
        }
        Ok(())
    }
}

impl ClipboardServiceImpl {
    async fn revision(&self, owner: &Owner) -> Result<i64, RuntimeError> {
        Ok(sqlx::query_scalar(
            "SELECT revision FROM clipboard_heads WHERE tenant_id=$1 AND user_id=$2",
        )
        .bind(&owner.tenant)
        .bind(&owner.user)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }
    async fn assemble(&self, owner: &Owner, seq: i64) -> Result<Vec<u8>, RuntimeError> {
        let rows=sqlx::query("SELECT idx,ciphertext FROM clipboard_chunks WHERE tenant_id=$1 AND user_id=$2 AND seq=$3 ORDER BY idx").bind(&owner.tenant).bind(&owner.user).bind(seq).fetch_all(&self.pool).await?;
        let scope = util::scope(owner);
        let mut data = Vec::new();
        for row in rows {
            let index: i32 = row.try_get("idx")?;
            let ciphertext: Vec<u8> = row.try_get("ciphertext")?;
            data.extend(self.keyring.open(
                &scope,
                &util::purpose(owner, seq, index as usize),
                &ciphertext,
            )?);
        }
        Ok(data)
    }
}
