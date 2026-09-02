impl ProgramStore {
    pub async fn reconcile_program_models(
        &self,
        program_id: &str,
        definition: &ProgramDefinition,
        image: &ProgramImage,
    ) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("开始同步 ProgramGraph 模型事务失败")?;
        for model in &definition.models {
            let model_symbol = model.id.to_string();
            let audit_metadata_json =
                serde_json::to_string(&model.audit).context("序列化模型审计配置失败")?;
            let primary_key_generation = model.primary_key.generation.as_str();
            let conflicting_symbol = sqlx::query_scalar::<_, String>(
                "SELECT program_symbol_id FROM engine_meta_models
                 WHERE name = $1 AND program_symbol_id IS NOT NULL AND program_symbol_id <> $2
                 LIMIT 1",
            )
            .bind(&model.name)
            .bind(&model_symbol)
            .fetch_optional(&mut *transaction)
            .await
            .with_context(|| format!("检查动态模型名称冲突失败: {}", model.name))?;
            if let Some(conflicting_symbol) = conflicting_symbol {
                bail!(
                    "动态模型名称已由其他 SymbolId 使用: {} ({conflicting_symbol})",
                    model.name
                );
            }
            let existing_primary_key_generation = sqlx::query_scalar::<_, String>(
                "SELECT primary_key_generation FROM engine_meta_models
                 WHERE program_symbol_id = $1
                    OR (program_symbol_id IS NULL AND name = $2)
                 LIMIT 1",
            )
            .bind(&model_symbol)
            .bind(&model.name)
            .fetch_optional(&mut *transaction)
            .await
            .with_context(|| format!("读取动态模型主键策略失败: {}", model.name))?;
            if existing_primary_key_generation
                .as_deref()
                .is_some_and(|value| value != primary_key_generation)
            {
                let has_records = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM engine_data_records WHERE model_name = $1)",
                )
                .bind(&model.name)
                .fetch_one(&mut *transaction)
                .await
                .with_context(|| format!("检查模型历史记录失败: {}", model.name))?;
                if has_records {
                    bail!("模型 {} 已存在业务记录，不能切换主键生成策略", model.title);
                }
            }
            let updated = sqlx::query(
                "UPDATE engine_meta_models
                 SET program_symbol_id = $1, name = $2, display_name = $3,
                     audit_metadata_json = $4, primary_key_generation = $5, updated_at_ms = $6
                 WHERE program_symbol_id = $1
                    OR (program_symbol_id IS NULL AND name = $2)",
            )
            .bind(&model_symbol)
            .bind(&model.name)
            .bind(&model.title)
            .bind(&audit_metadata_json)
            .bind(primary_key_generation)
            .bind(timestamp_ms())
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("同步动态模型失败: {}", model.name))?;
            if updated.rows_affected() > 1 {
                bail!("动态模型名称存在多个历史对象: {}", model.name);
            }
            if updated.rows_affected() == 0 {
                sqlx::query(
                    "INSERT INTO engine_meta_models
                     (id, name, display_name, created_at_ms, updated_at_ms, program_symbol_id,
                      audit_metadata_json, primary_key_generation)
                     VALUES ($1, $2, $3, $4, $4, $1, $5, $6)",
                )
                .bind(&model_symbol)
                .bind(&model.name)
                .bind(&model.title)
                .bind(timestamp_ms())
                .bind(&audit_metadata_json)
                .bind(primary_key_generation)
                .execute(&mut *transaction)
                .await
                .with_context(|| format!("创建动态模型失败: {}", model.name))?;
            }
            for (order, field) in model.fields.iter().enumerate() {
                let field_symbol = field.id.to_string();
                let field_type = engine_field_type(&field.value_type);
                let domain_metadata_json =
                    serde_json::to_string(&field.options).context("序列化字段低代码配置失败")?;
                let validation_json = serde_json::to_string(&field.options.validation)
                    .context("序列化字段校验配置失败")?;
                let updated = sqlx::query(
                    "UPDATE engine_meta_fields
                     SET program_symbol_id = $1, model_name = $2, name = $3, display_name = $4,
                         field_type = $5, is_required = $6, order_index = $7,
                         domain_metadata_json = $8, validation_json = $9, updated_at_ms = $10
                     WHERE program_symbol_id = $1
                        OR (program_symbol_id IS NULL AND model_name = $2 AND name = $3)",
                )
                .bind(&field_symbol)
                .bind(&model.name)
                .bind(&field.name)
                .bind(&field.title)
                .bind(field_type)
                .bind(field.required)
                .bind(order as i32)
                .bind(&domain_metadata_json)
                .bind(&validation_json)
                .bind(timestamp_ms())
                .execute(&mut *transaction)
                .await
                .with_context(|| format!("同步动态字段失败: {}.{}", model.name, field.name))?;
                if updated.rows_affected() > 1 {
                    bail!(
                        "动态字段名称存在多个历史对象: {}.{}",
                        model.name,
                        field.name
                    );
                }
                if updated.rows_affected() == 0 {
                    sqlx::query(
                        "INSERT INTO engine_meta_fields
                         (id, model_name, name, display_name, field_type, is_required, expression,
                          dependency_json, order_index, created_at_ms, updated_at_ms,
                          domain_metadata_json, validation_json, program_symbol_id)
                         VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, $7, $8, $8, $9, $10, $1)",
                    )
                    .bind(&field_symbol)
                    .bind(&model.name)
                    .bind(&field.name)
                    .bind(&field.title)
                    .bind(field_type)
                    .bind(field.required)
                    .bind(order as i32)
                    .bind(timestamp_ms())
                    .bind(&domain_metadata_json)
                    .bind(&validation_json)
                    .execute(&mut *transaction)
                    .await
                    .with_context(|| format!("创建动态字段失败: {}.{}", model.name, field.name))?;
                }
            }
        }

        reconcile_expression_indexes(&mut transaction, program_id, image).await?;
        transaction
            .commit()
            .await
            .context("提交 ProgramGraph 模型同步事务失败")?;
        Ok(())
    }

    pub async fn activate_revision(&self, program_id: &str, revision_id: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE engine_programs
             SET active_revision_id = $1, updated_at_ms = $2
             WHERE id = $3
               AND EXISTS (
                   SELECT 1 FROM engine_program_revisions revision
                   WHERE revision.id = $1 AND revision.program_id = $3
               )",
        )
        .bind(revision_id)
        .bind(timestamp_ms())
        .bind(program_id)
        .execute(&self.pool)
        .await
        .context("激活 program revision 失败")?;
        if result.rows_affected() != 1 {
            bail!("revision not found in program: {revision_id}");
        }
        Ok(())
    }

    pub async fn rollback(
        &self,
        program_id: &str,
        source_revision_id: &str,
    ) -> Result<RevisionSnapshot> {
        let source = self.revision(source_revision_id).await?;
        if source.program_id != program_id {
            bail!("revision not found in program: {source_revision_id}");
        }
        let mut transaction = self.pool.begin().await.context("开始 rollback 事务失败")?;
        let current_version = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM engine_program_drafts
             WHERE program_id = $1 FOR UPDATE",
        )
        .bind(program_id)
        .fetch_one(&mut *transaction)
        .await
        .context("锁定 rollback Draft 失败")?;
        sqlx::query(
            "UPDATE engine_program_drafts
             SET version = $1, definition = $2, updated_at_ms = $3
             WHERE program_id = $4",
        )
        .bind(current_version + 1)
        .bind(Json(&source.definition))
        .bind(timestamp_ms())
        .bind(program_id)
        .execute(&mut *transaction)
        .await
        .context("回写 rollback Draft 失败")?;
        let revision = self
            .insert_revision(
                &mut transaction,
                program_id,
                source.definition,
                "rollback",
                &Value::Array(Vec::new()),
            )
            .await?;
        transaction
            .commit()
            .await
            .context("提交 rollback 事务失败")?;
        Ok(revision)
    }

    pub async fn start_revision_run(&self, program_id: &str) -> Result<RevisionRunSnapshot> {
        let id = Uuid::new_v4().to_string();
        let now = timestamp_ms();
        sqlx::query(
            "INSERT INTO engine_revision_runs
             (id, program_id, revision_id, status, stage, diagnostics, tests,
              started_at_ms, finished_at_ms, duration_ms)
             VALUES ($1, $2, NULL, 'running', 'schema', '[]'::jsonb, '[]'::jsonb, $3, 0, 0)",
        )
        .bind(&id)
        .bind(program_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("创建 revision run 失败")?;
        Ok(RevisionRunSnapshot {
            id,
            program_id: program_id.to_owned(),
            revision_id: None,
            status: "running".to_owned(),
            stage: "schema".to_owned(),
            diagnostics: Value::Array(Vec::new()),
            tests: Value::Array(Vec::new()),
            started_at_ms: now,
            finished_at_ms: 0,
            duration_ms: 0,
        })
    }

    pub async fn finish_revision_run(
        &self,
        run: &RevisionRunSnapshot,
        revision_id: Option<&str>,
        succeeded: bool,
        stage: &str,
        diagnostics: &Value,
        tests: &Value,
    ) -> Result<()> {
        let finished_at_ms = timestamp_ms();
        let duration_ms = finished_at_ms.saturating_sub(run.started_at_ms);
        sqlx::query(
            "UPDATE engine_revision_runs
             SET revision_id = $1, status = $2, stage = $3, diagnostics = $4, tests = $5,
                 finished_at_ms = $6, duration_ms = $7
             WHERE id = $8 AND status = 'running'",
        )
        .bind(revision_id)
        .bind(if succeeded { "succeeded" } else { "failed" })
        .bind(stage)
        .bind(Json(diagnostics))
        .bind(Json(tests))
        .bind(finished_at_ms)
        .bind(duration_ms)
        .bind(&run.id)
        .execute(&self.pool)
        .await
        .context("完成 revision run 失败")?;
        Ok(())
    }

    pub async fn create_vibe_session(
        &self,
        program_id: &str,
        base_version: i64,
    ) -> Result<VibeSessionSnapshot> {
        let id = Uuid::new_v4().to_string();
        let now = timestamp_ms();
        sqlx::query(
            "INSERT INTO engine_vibe_sessions
             (id, program_id, base_version, status, final_revision_id, created_at_ms, updated_at_ms)
             VALUES ($1, $2, $3, 'running', NULL, $4, $4)",
        )
        .bind(&id)
        .bind(program_id)
        .bind(base_version)
        .bind(now)
        .execute(&self.pool)
        .await
        .context("创建 vibe session 失败")?;
        Ok(VibeSessionSnapshot {
            id,
            program_id: program_id.to_owned(),
            base_version,
            status: "running".to_owned(),
            final_revision_id: None,
            diagnostics: Value::Array(Vec::new()),
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub async fn vibe_session(&self, session_id: &str) -> Result<Option<VibeSessionSnapshot>> {
        let row = sqlx::query(
            "SELECT sessions.id, sessions.program_id, sessions.base_version, sessions.status,
                    sessions.final_revision_id, sessions.created_at_ms, sessions.updated_at_ms,
                    COALESCE((
                        SELECT messages.diagnostics
                        FROM engine_vibe_messages messages
                        WHERE messages.session_id = sessions.id
                          AND messages.role = 'gate'
                        ORDER BY messages.sequence DESC
                        LIMIT 1
                    ), '[]'::jsonb) AS diagnostics
             FROM engine_vibe_sessions sessions
             WHERE sessions.id = $1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .context("查询 vibe session 失败")?;
        row.as_ref().map(vibe_session_from_row).transpose()
    }

    pub async fn append_vibe_message(
        &self,
        session_id: &str,
        input: &VibeMessageInput,
    ) -> Result<String> {
        validate_vibe_role(&input.role)?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO engine_vibe_messages
             (id, session_id, sequence, role, prompt, model, input_tokens, output_tokens,
              patch, diagnostics, tests, created_at_ms)
             SELECT $1, $2, COALESCE(MAX(sequence), -1) + 1, $3, $4, $5, $6, $7,
                    $8, $9, $10, $11
             FROM engine_vibe_messages WHERE session_id = $2",
        )
        .bind(&id)
        .bind(session_id)
        .bind(&input.role)
        .bind(&input.prompt)
        .bind(&input.model)
        .bind(input.input_tokens)
        .bind(input.output_tokens)
        .bind(input.patch.as_ref().map(Json))
        .bind(Json(&input.diagnostics))
        .bind(Json(&input.tests))
        .bind(timestamp_ms())
        .execute(&self.pool)
        .await
        .context("保存 vibe message 失败")?;
        Ok(id)
    }

    pub async fn finish_vibe_session(
        &self,
        session_id: &str,
        revision_id: Option<&str>,
        succeeded: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE engine_vibe_sessions
             SET status = $1, final_revision_id = $2, updated_at_ms = $3
             WHERE id = $4 AND status = 'running'",
        )
        .bind(if succeeded { "succeeded" } else { "failed" })
        .bind(revision_id)
        .bind(timestamp_ms())
        .bind(session_id)
        .execute(&self.pool)
        .await
        .context("完成 vibe session 失败")?;
        Ok(())
    }
}
