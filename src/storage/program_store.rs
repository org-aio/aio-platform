use std::{collections::BTreeSet, fmt};

use crate::{
    DraftSnapshot, GraphPatchBatch, ImageTarget, ProgramDefinition, ProgramImage,
    RevisionRunSnapshot, RevisionSnapshot, StudioPageParams, ValueType, VibeMessageInput,
    VibeSessionSnapshot,
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions, types::Json};
use uuid::Uuid;

use crate::studio_contract::StudioPage as StudioPageData;
use az_plugin_core::{timestamp_ms, verify_database_url};

#[derive(Clone)]
pub struct ProgramStore {
    pool: PgPool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramState {
    pub id: String,
    pub name: String,
    pub title: String,
    pub active_revision_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftVersionConflict {
    pub expected: i64,
    pub actual: i64,
}

impl fmt::Display for DraftVersionConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "draft version conflict: expected {}, actual {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for DraftVersionConflict {}

impl ProgramStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let database_url = verify_database_url(database_url)?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .context("连接 ProgramGraph PostgreSQL 失败")?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn active_program(&self) -> Result<Option<ProgramState>> {
        let row = sqlx::query(
            "SELECT id, name, title, active_revision_id, created_at_ms, updated_at_ms
             FROM engine_programs
             WHERE singleton AND active_revision_id IS NOT NULL",
        )
        .fetch_optional(&self.pool)
        .await
        .context("查询活动 Program 失败")?;
        row.as_ref().map(program_from_row).transpose()
    }

    pub async fn program(&self) -> Result<ProgramState> {
        let row = sqlx::query(
            "SELECT id, name, title, active_revision_id, created_at_ms, updated_at_ms
             FROM engine_programs WHERE singleton",
        )
        .fetch_optional(&self.pool)
        .await
        .context("查询 engine program 失败")?
        .context("Program 不存在")?;
        program_from_row(&row)
    }

    pub async fn draft(&self) -> Result<DraftSnapshot> {
        let row = sqlx::query(
            "SELECT draft.program_id, draft.version, draft.definition, draft.updated_at_ms
             FROM engine_program_drafts draft
             INNER JOIN engine_programs program ON program.id = draft.program_id
             WHERE program.singleton",
        )
        .fetch_optional(&self.pool)
        .await
        .context("查询 engine program draft 失败")?
        .context("Program Draft 不存在")?;
        draft_from_row(&row)
    }

    pub async fn patch_draft(&self, batch: &GraphPatchBatch) -> Result<DraftSnapshot> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("开始 Draft Patch 事务失败")?;
        let row = sqlx::query(
            "SELECT draft.program_id, draft.version, draft.definition, draft.updated_at_ms
             FROM engine_program_drafts draft
             INNER JOIN engine_programs program ON program.id = draft.program_id
             WHERE program.singleton
             FOR UPDATE",
        )
        .fetch_optional(&mut *transaction)
        .await
        .context("锁定 engine program draft 失败")?
        .context("Program Draft 不存在")?;
        let mut draft = draft_from_row(&row)?;
        let program_id = draft.program_id.clone();
        if draft.version != batch.base_version {
            return Err(DraftVersionConflict {
                expected: batch.base_version,
                actual: draft.version,
            }
            .into());
        }
        let previous_definition = draft.definition.clone();
        draft
            .definition
            .apply_patch_batch(batch)
            .context("应用 GraphPatchBatch 失败")?;
        for previous_model in &previous_definition.models {
            let Some(next_model) = draft
                .definition
                .models
                .iter()
                .find(|model| model.id == previous_model.id)
            else {
                continue;
            };
            if previous_model.primary_key == next_model.primary_key {
                continue;
            }
            let has_records = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM engine_data_records WHERE model_name = $1)",
            )
            .bind(&previous_model.name)
            .fetch_one(&mut *transaction)
            .await
            .with_context(|| format!("检查模型历史记录失败: {}", previous_model.name))?;
            if has_records {
                bail!(
                    "模型 {} 已存在业务记录，不能切换主键生成策略",
                    previous_model.title
                );
            }
        }
        draft.version += 1;
        draft.updated_at_ms = timestamp_ms();
        let result = sqlx::query(
            "UPDATE engine_program_drafts
             SET version = $1, definition = $2, updated_at_ms = $3
             WHERE program_id = $4 AND version = $5",
        )
        .bind(draft.version)
        .bind(Json(&draft.definition))
        .bind(draft.updated_at_ms)
        .bind(&program_id)
        .bind(batch.base_version)
        .execute(&mut *transaction)
        .await
        .context("更新 engine program draft 失败")?;
        if result.rows_affected() != 1 {
            bail!("draft version conflict: concurrent update");
        }
        transaction
            .commit()
            .await
            .context("提交 Draft Patch 事务失败")?;
        Ok(draft)
    }

    pub async fn create_revision_from_draft(
        &self,
        program_id: &str,
        origin: &str,
        diagnostics: &Value,
    ) -> Result<RevisionSnapshot> {
        validate_revision_origin(origin)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("开始创建 revision 事务失败")?;
        let draft_row = sqlx::query(
            "SELECT program_id, version, definition, updated_at_ms
             FROM engine_program_drafts
             WHERE program_id = $1
             FOR SHARE",
        )
        .bind(program_id)
        .fetch_optional(&mut *transaction)
        .await
        .context("读取待发布 Draft 失败")?
        .with_context(|| format!("program draft not found: {program_id}"))?;
        let draft = draft_from_row(&draft_row)?;
        let revision = self
            .insert_revision(
                &mut transaction,
                program_id,
                draft.definition,
                origin,
                diagnostics,
            )
            .await?;
        transaction
            .commit()
            .await
            .context("提交 program revision 失败")?;
        Ok(revision)
    }

    pub async fn create_revision(
        &self,
        program_id: &str,
        definition: ProgramDefinition,
        origin: &str,
        diagnostics: &Value,
    ) -> Result<RevisionSnapshot> {
        validate_revision_origin(origin)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("开始创建 revision 事务失败")?;
        let revision = self
            .insert_revision(
                &mut transaction,
                program_id,
                definition,
                origin,
                diagnostics,
            )
            .await?;
        transaction
            .commit()
            .await
            .context("提交 program revision 失败")?;
        Ok(revision)
    }

    async fn insert_revision(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        program_id: &str,
        definition: ProgramDefinition,
        origin: &str,
        diagnostics: &Value,
    ) -> Result<RevisionSnapshot> {
        sqlx::query("SELECT id FROM engine_programs WHERE id = $1 FOR UPDATE")
            .bind(program_id)
            .fetch_optional(&mut **transaction)
            .await
            .context("锁定 program revision 序号失败")?
            .with_context(|| format!("program not found: {program_id}"))?;
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(revision), 0) + 1
             FROM engine_program_revisions
             WHERE program_id = $1",
        )
        .bind(program_id)
        .fetch_one(&mut **transaction)
        .await
        .context("分配 program revision 序号失败")?;
        let id = Uuid::new_v4().to_string();
        let content_hash =
            crate::content_hash(&definition).context("计算 ProgramDefinition 内容哈希失败")?;
        let now = timestamp_ms();
        sqlx::query(
            "INSERT INTO engine_program_revisions
             (id, program_id, revision, definition, content_hash, origin, diagnostics, created_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&id)
        .bind(program_id)
        .bind(revision)
        .bind(Json(&definition))
        .bind(&content_hash)
        .bind(origin)
        .bind(Json(diagnostics))
        .bind(now)
        .execute(&mut **transaction)
        .await
        .context("保存不可变 program revision 失败")?;
        Ok(RevisionSnapshot {
            id,
            program_id: program_id.to_owned(),
            revision,
            definition,
            content_hash,
            origin: origin.to_owned(),
            diagnostics: diagnostics.clone(),
            created_at_ms: now,
        })
    }

    pub async fn revisions(
        &self,
        program_id: &str,
        page: StudioPageParams,
    ) -> Result<StudioPageData<RevisionSnapshot>> {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM engine_program_revisions WHERE program_id = $1",
        )
        .bind(program_id)
        .fetch_one(&self.pool)
        .await
        .context("统计 program revisions 失败")?;
        let rows = sqlx::query(
            "SELECT id, program_id, revision, definition, content_hash, origin,
                    diagnostics, created_at_ms
             FROM engine_program_revisions
             WHERE program_id = $1
             ORDER BY revision DESC
             OFFSET $2 LIMIT $3",
        )
        .bind(program_id)
        .bind(page.o as i64)
        .bind(page.s as i64)
        .fetch_all(&self.pool)
        .await
        .context("查询 program revisions 失败")?;
        Ok(StudioPageData {
            d: rows
                .iter()
                .map(revision_from_row)
                .collect::<Result<Vec<_>>>()?,
            t: total as u64,
            p: page,
        })
    }

    pub async fn revision(&self, revision_id: &str) -> Result<RevisionSnapshot> {
        let row = sqlx::query(
            "SELECT id, program_id, revision, definition, content_hash, origin,
                    diagnostics, created_at_ms
             FROM engine_program_revisions WHERE id = $1",
        )
        .bind(revision_id)
        .fetch_optional(&self.pool)
        .await
        .context("查询 program revision 失败")?
        .with_context(|| format!("revision not found: {revision_id}"))?;
        revision_from_row(&row)
    }

    pub async fn save_image(&self, image: &ProgramImage) -> Result<()> {
        let bytes = image.encode().context("序列化 ProgramImage 失败")?;
        sqlx::query(
            "INSERT INTO engine_program_images
             (content_hash, compiler_version, target, revision_id, image, created_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (content_hash, compiler_version, target) DO NOTHING",
        )
        .bind(&image.content_hash)
        .bind(&image.compiler_version)
        .bind(image.target.as_str())
        .bind(&image.revision_id)
        .bind(bytes)
        .bind(timestamp_ms())
        .execute(&self.pool)
        .await
        .context("保存 ProgramImage cache 失败")?;
        Ok(())
    }

    pub async fn image(
        &self,
        content_hash: &str,
        compiler_version: &str,
        target: ImageTarget,
    ) -> Result<Option<ProgramImage>> {
        let bytes = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT image FROM engine_program_images
             WHERE content_hash = $1 AND compiler_version = $2 AND target = $3",
        )
        .bind(content_hash)
        .bind(compiler_version)
        .bind(target.as_str())
        .fetch_optional(&self.pool)
        .await
        .context("读取 ProgramImage cache 失败")?;
        bytes
            .map(|value| ProgramImage::decode(&value).context("反序列化 ProgramImage 失败"))
            .transpose()
    }
}

include!("store_lifecycle.rs");

async fn reconcile_expression_indexes(
    transaction: &mut Transaction<'_, Postgres>,
    program_id: &str,
    image: &ProgramImage,
) -> Result<()> {
    let mut desired = BTreeSet::new();
    for model in image.models.values() {
        for (slot, options) in &model.field_options {
            if !options.unique {
                continue;
            }
            let Some(field) = model.field_names.get(slot) else {
                continue;
            };
            let index_name =
                managed_unique_index_name(program_id, &model.id.to_string(), &model.name, field);
            let statement = format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {index_name} ON engine_data_records ((payload -> '{}')) \
                 WHERE model_name = '{}' AND payload ? '{}' AND payload -> '{}' <> 'null'::jsonb",
                quote_literal(field),
                quote_literal(&model.name),
                quote_literal(field),
                quote_literal(field),
            );
            sqlx::query(&statement)
                .execute(&mut **transaction)
                .await
                .with_context(|| format!("创建字段唯一索引失败: {index_name}"))?;
            sqlx::query(
                "INSERT INTO engine_program_expression_indexes
                 (program_id, index_name, model_symbol_id, field_slots, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (program_id, index_name) DO NOTHING",
            )
            .bind(program_id)
            .bind(&index_name)
            .bind(model.id.to_string())
            .bind(Json(vec![slot.to_string()]))
            .bind(timestamp_ms())
            .execute(&mut **transaction)
            .await
            .context("登记字段唯一索引失败")?;
            desired.insert(index_name);
        }
        for (slot, options) in &model.field_options {
            if !options.filterable && !model.field_relations.contains_key(slot) {
                continue;
            }
            let Some(field) = model.field_names.get(slot) else {
                continue;
            };
            let index_name = managed_index_name(
                program_id,
                &model.id.to_string(),
                &model.name,
                &["filter".to_owned(), field.clone()],
            );
            let statement = format!(
                "CREATE INDEX IF NOT EXISTS {index_name} ON engine_data_records \
                 ((payload ->> '{}')) WHERE model_name = '{}'",
                quote_literal(field),
                quote_literal(&model.name),
            );
            sqlx::query(&statement)
                .execute(&mut **transaction)
                .await
                .with_context(|| format!("创建字段筛选索引失败: {index_name}"))?;
            sqlx::query(
                "INSERT INTO engine_program_expression_indexes
                 (program_id, index_name, model_symbol_id, field_slots, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (program_id, index_name) DO NOTHING",
            )
            .bind(program_id)
            .bind(&index_name)
            .bind(model.id.to_string())
            .bind(Json(vec![slot.to_string()]))
            .bind(timestamp_ms())
            .execute(&mut **transaction)
            .await
            .context("登记字段筛选索引失败")?;
            desired.insert(index_name);
        }
        for index in &model.expression_indexes {
            let fields = index
                .fields
                .iter()
                .filter_map(|slot| model.field_names.get(slot))
                .cloned()
                .collect::<Vec<_>>();
            if fields.is_empty() {
                continue;
            }
            let mut identity_fields = Vec::with_capacity(fields.len() + 1);
            identity_fields.push(if index.unique {
                "unique".to_owned()
            } else {
                "index".to_owned()
            });
            identity_fields.extend(fields.iter().cloned());
            let index_name = managed_index_name(
                program_id,
                &model.id.to_string(),
                &model.name,
                &identity_fields,
            );
            let expressions = fields
                .iter()
                .map(|field| format!("(payload ->> '{}')", quote_literal(field)))
                .collect::<Vec<_>>()
                .join(", ");
            let statement = format!(
                "CREATE {}INDEX IF NOT EXISTS {index_name} ON engine_data_records ({expressions}) \
                 WHERE model_name = '{}'",
                if index.unique { "UNIQUE " } else { "" },
                quote_literal(&model.name),
            );
            sqlx::query(&statement)
                .execute(&mut **transaction)
                .await
                .with_context(|| format!("创建 ProgramGraph 表达式索引失败: {index_name}"))?;
            sqlx::query(
                "INSERT INTO engine_program_expression_indexes
                 (program_id, index_name, model_symbol_id, field_slots, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (program_id, index_name) DO NOTHING",
            )
            .bind(program_id)
            .bind(&index_name)
            .bind(model.id.to_string())
            .bind(Json(
                index
                    .fields
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ))
            .bind(timestamp_ms())
            .execute(&mut **transaction)
            .await
            .context("登记 ProgramGraph 表达式索引失败")?;
            desired.insert(index_name);
        }
    }
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT index_name FROM engine_program_expression_indexes WHERE program_id = $1",
    )
    .bind(program_id)
    .fetch_all(&mut **transaction)
    .await
    .context("读取 ProgramGraph 表达式索引登记失败")?;
    for index_name in existing {
        if desired.contains(&index_name) {
            continue;
        }
        if !is_managed_index_name(&index_name) {
            bail!("拒绝删除非 ProgramGraph 管理索引: {index_name}");
        }
        sqlx::query(&format!("DROP INDEX IF EXISTS {index_name}"))
            .execute(&mut **transaction)
            .await
            .with_context(|| format!("删除过期 ProgramGraph 表达式索引失败: {index_name}"))?;
        sqlx::query(
            "DELETE FROM engine_program_expression_indexes
             WHERE program_id = $1 AND index_name = $2",
        )
        .bind(program_id)
        .bind(index_name)
        .execute(&mut **transaction)
        .await
        .context("删除 ProgramGraph 表达式索引登记失败")?;
    }
    Ok(())
}

fn managed_index_name(
    program_id: &str,
    model_id: &str,
    model_name: &str,
    fields: &[String],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(program_id.as_bytes());
    hasher.update(model_id.as_bytes());
    hasher.update(model_name.as_bytes());
    for field in fields {
        hasher.update(field.as_bytes());
        hasher.update([0]);
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("engine_program_{}_idx", &digest[..24])
}

fn managed_unique_index_name(
    program_id: &str,
    model_id: &str,
    model_name: &str,
    field: &str,
) -> String {
    managed_index_name(
        program_id,
        model_id,
        model_name,
        &["unique".to_owned(), field.to_owned()],
    )
}

fn is_managed_index_name(value: &str) -> bool {
    value.starts_with("engine_program_")
        && value.ends_with("_idx")
        && value
            .trim_start_matches("engine_program_")
            .trim_end_matches("_idx")
            .bytes()
            .all(|value| value.is_ascii_hexdigit())
}

fn quote_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn engine_field_type(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Boolean => "boolean",
        ValueType::Integer => "int",
        ValueType::Decimal => "decimal",
        ValueType::TimestampMs => "datetime",
        ValueType::Text | ValueType::File => "string",
        ValueType::Any
        | ValueType::Null
        | ValueType::Object { .. }
        | ValueType::List { .. }
        | ValueType::Optional { .. } => "json",
    }
}

fn program_from_row(row: &sqlx::postgres::PgRow) -> Result<ProgramState> {
    Ok(ProgramState {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        title: row.try_get("title")?,
        active_revision_id: row.try_get("active_revision_id")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn draft_from_row(row: &sqlx::postgres::PgRow) -> Result<DraftSnapshot> {
    let definition: Json<ProgramDefinition> = row.try_get("definition")?;
    Ok(DraftSnapshot {
        program_id: row.try_get("program_id")?,
        version: row.try_get("version")?,
        definition: definition.0,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn revision_from_row(row: &sqlx::postgres::PgRow) -> Result<RevisionSnapshot> {
    let definition: Json<ProgramDefinition> = row.try_get("definition")?;
    let diagnostics: Json<Value> = row.try_get("diagnostics")?;
    Ok(RevisionSnapshot {
        id: row.try_get("id")?,
        program_id: row.try_get("program_id")?,
        revision: row.try_get("revision")?,
        definition: definition.0,
        content_hash: row.try_get("content_hash")?,
        origin: row.try_get("origin")?,
        diagnostics: diagnostics.0,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

fn vibe_session_from_row(row: &sqlx::postgres::PgRow) -> Result<VibeSessionSnapshot> {
    let diagnostics: Json<Value> = row.try_get("diagnostics")?;
    Ok(VibeSessionSnapshot {
        id: row.try_get("id")?,
        program_id: row.try_get("program_id")?,
        base_version: row.try_get("base_version")?,
        status: row.try_get("status")?,
        final_revision_id: row.try_get("final_revision_id")?,
        diagnostics: diagnostics.0,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn validate_revision_origin(value: &str) -> Result<()> {
    if matches!(value, "studio" | "vibe" | "migration" | "rollback") {
        Ok(())
    } else {
        bail!("无效 revision origin: {value}")
    }
}

fn validate_vibe_role(value: &str) -> Result<()> {
    if matches!(value, "user" | "agent" | "gate") {
        Ok(())
    } else {
        bail!("无效 vibe message role: {value}")
    }
}
