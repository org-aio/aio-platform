//! ProgramGraph 正式持久化契约的 PostgreSQL 集成测试。

use az_plugin_core::verify_database_url;
use az_studio::{
    CapabilityCatalog, GraphEntity, GraphPatch, GraphPatchBatch, ImageTarget, ModelDefinition,
    ModelIndexDefinition, PatchOrigin, ProgramCompiler, StudioPageParams, SymbolId, ValueType,
    program_store::{DraftVersionConflict, ProgramStore},
};
use serde_json::json;
use sqlx::postgres::PgListener;

#[tokio::test]
async fn program_store_enforces_versions_revisions_cache_and_activation() -> anyhow::Result<()> {
    let Some(database_url) = test_database_url()? else {
        return Ok(());
    };
    let store = ProgramStore::connect(&database_url).await?;
    let suffix = unique_suffix();
    let program = store.program().await?;
    let initial = store.draft().await?;
    let model_id = SymbolId::new();
    let field_id = SymbolId::new();
    let index_id = SymbolId::new();
    let model_name = format!(
        "program_model_{}_{}",
        std::process::id(),
        az_plugin_core::timestamp_ms()
    );

    let patched = store
        .patch_draft(&GraphPatchBatch {
            base_version: initial.version,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::Rename {
                    target_id: initial.definition.id,
                    name: format!("program-renamed-{suffix}"),
                    title: Some("已发布程序".to_owned()),
                },
                GraphPatch::Insert {
                    parent_id: initial.definition.id,
                    collection: az_studio::ChildCollection::Models,
                    index: 0,
                    entity: Box::new(GraphEntity::Model(ModelDefinition {
                        id: model_id,
                        name: model_name.clone(),
                        title: "程序模型".to_owned(),
                        state: az_studio::DefinitionState::Known,
                        primary_key: az_studio::ModelPrimaryKeyDefinition {
                            generation: az_studio::PrimaryKeyGeneration::AutoIncrement,
                        },
                        fields: vec![az_studio::FieldDefinition {
                            id: field_id,
                            name: "serial_number".to_owned(),
                            title: "序列号".to_owned(),
                            value_type: ValueType::Text,
                            state: az_studio::DefinitionState::Known,
                            required: true,
                            options: az_studio::FieldOptions {
                                unique: true,
                                ..az_studio::FieldOptions::default()
                            },
                            relation: None,
                        }],
                        indexes: vec![ModelIndexDefinition {
                            id: index_id,
                            fields: vec![field_id],
                            unique: false,
                        }],
                        queries: Vec::new(),
                        validations: Vec::new(),
                        audit: az_studio::ModelAuditDefinition::default(),
                    })),
                },
            ],
        })
        .await?;
    assert_eq!(patched.version, initial.version + 1);
    let conflict = store
        .patch_draft(&GraphPatchBatch {
            base_version: initial.version,
            origin: PatchOrigin::Studio,
            patches: Vec::new(),
        })
        .await
        .expect_err("旧版本 Patch 必须冲突");
    assert_eq!(
        conflict
            .downcast_ref::<DraftVersionConflict>()
            .map(|value| value.actual),
        Some(patched.version)
    );

    let revision = store
        .create_revision_from_draft(&program.id, "studio", &json!([]))
        .await?;
    let capabilities = CapabilityCatalog::default();
    let mut image = ProgramCompiler::new("integration-v1", &capabilities).compile(
        &revision.definition,
        &revision.id,
        ImageTarget::Universal,
    )?;
    image.revision_id.clone_from(&revision.id);
    store
        .reconcile_program_models(&program.id, &revision.definition, &image)
        .await?;
    store.save_image(&image).await?;
    assert!(
        store
            .image(
                &image.content_hash,
                "integration-v1",
                ImageTarget::Universal
            )
            .await?
            .is_some()
    );
    let linked_model = sqlx::query_scalar::<_, String>(
        "SELECT program_symbol_id FROM engine_meta_models WHERE name = $1",
    )
    .bind(&model_name)
    .fetch_one(store.pool())
    .await?;
    assert_eq!(linked_model, model_id.to_string());
    let primary_key_generation = sqlx::query_scalar::<_, String>(
        "SELECT primary_key_generation FROM engine_meta_models WHERE name = $1",
    )
    .bind(&model_name)
    .fetch_one(store.pool())
    .await?;
    assert_eq!(primary_key_generation, "auto_increment");
    let linked_field = sqlx::query_scalar::<_, String>(
        "SELECT program_symbol_id FROM engine_meta_fields
         WHERE model_name = $1 AND name = 'serial_number'",
    )
    .bind(&model_name)
    .fetch_one(store.pool())
    .await?;
    assert_eq!(linked_field, field_id.to_string());
    let managed_index = sqlx::query_scalar::<_, String>(
        "SELECT index_name FROM engine_program_expression_indexes WHERE program_id = $1",
    )
    .bind(&program.id)
    .fetch_one(store.pool())
    .await?;
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
            .bind(&managed_index)
            .fetch_one(store.pool())
            .await?
    );
    let managed_index_definitions = sqlx::query_scalar::<_, String>(
        "SELECT indexdef FROM pg_indexes
         WHERE schemaname = current_schema()
           AND indexname IN (
               SELECT index_name FROM engine_program_expression_indexes
               WHERE program_id = $1
           )",
    )
    .bind(&program.id)
    .fetch_all(store.pool())
    .await?;
    assert!(
        managed_index_definitions
            .iter()
            .any(|definition| definition.starts_with("CREATE UNIQUE INDEX"))
    );
    assert!(
        store
            .image(
                &image.content_hash,
                "integration-v2",
                ImageTarget::Universal
            )
            .await?
            .is_none()
    );

    let mut listener = PgListener::connect(&database_url).await?;
    listener.listen("engine_program_activated").await?;
    store.activate_revision(&program.id, &revision.id).await?;
    let notification =
        tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv()).await??;
    let payload: serde_json::Value = serde_json::from_str(notification.payload())?;
    assert_eq!(payload["revision_id"], revision.id);

    let immutable_update =
        sqlx::query("UPDATE engine_program_revisions SET origin = 'rollback' WHERE id = $1")
            .bind(&revision.id)
            .execute(store.pool())
            .await
            .expect_err("Revision 更新必须被数据库拒绝");
    assert!(
        immutable_update
            .to_string()
            .contains("immutable program row")
    );

    let rollback = store.rollback(&program.id, &revision.id).await?;
    assert!(rollback.revision > revision.revision);
    assert_eq!(store.draft().await?.definition, rollback.definition);
    let revisions = store
        .revisions(&program.id, StudioPageParams { o: 0, s: 10 })
        .await?;
    assert!(revisions.t >= 2);
    assert_eq!(revisions.d[0].origin, "rollback");

    let index_names = sqlx::query_scalar::<_, String>(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname = current_schema()
           AND tablename = 'engine_program_revisions'",
    )
    .fetch_all(store.pool())
    .await?;
    assert!(
        index_names
            .iter()
            .any(|name| name == "engine_program_revisions_number_uidx")
    );
    let preserved_records =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM engine_data_records")
            .fetch_one(store.pool())
            .await?;
    assert!(preserved_records >= 0);
    Ok(())
}

fn test_database_url() -> anyhow::Result<Option<String>> {
    let value =
        std::env::var("AZ_AIO_TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"));
    match value {
        Ok(value) => Ok(Some(verify_database_url(&value)?.to_string())),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn unique_suffix() -> String {
    format!("{}-{}", std::process::id(), az_plugin_core::timestamp_ms())
}
