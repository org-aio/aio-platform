use crate::{
    AuditFieldKind, ChildCollection, DefinitionState, EditableProperty, FieldDefinition,
    FieldOptions, GraphEntity, GraphPatch, ModelAuditDefinition, ModelAuditField, ModelDefinition,
    SymbolId,
};

pub(crate) fn model_audit_patches(
    model: &ModelDefinition,
    kind: AuditFieldKind,
    enabled: bool,
) -> Result<Vec<GraphPatch>, String> {
    let existing_binding = model.audit.fields.iter().find(|field| field.kind == kind);
    if existing_binding.is_some() == enabled {
        return Ok(Vec::new());
    }

    let mut audit_fields = model.audit.fields.clone();
    let mut patches = Vec::new();
    if enabled {
        let field_id = existing_or_new_audit_field(model, kind, &mut patches)?;
        audit_fields.push(ModelAuditField { kind, field_id });
    } else {
        audit_fields.retain(|field| field.kind != kind);
    }
    audit_fields.sort_by_key(|field| field.kind);
    patches.push(GraphPatch::SetProperty {
        target_id: model.id,
        property: EditableProperty::ModelAudit,
        value: serde_json::json!(ModelAuditDefinition {
            fields: audit_fields,
        }),
    });
    Ok(patches)
}

fn existing_or_new_audit_field(
    model: &ModelDefinition,
    kind: AuditFieldKind,
    patches: &mut Vec<GraphPatch>,
) -> Result<SymbolId, String> {
    if let Some(field) = model
        .fields
        .iter()
        .find(|field| field.name == kind.default_name())
    {
        if field.value_type != kind.default_value_type() {
            return Err(format!(
                "审计字段 {} 类型不符合审计语义",
                kind.default_name(),
            ));
        }
        return Ok(field.id);
    }

    let field_id = SymbolId::new();
    patches.push(GraphPatch::Insert {
        parent_id: model.id,
        collection: ChildCollection::Fields,
        index: model.fields.len(),
        entity: Box::new(GraphEntity::Field(audit_field_definition(kind, field_id))),
    });
    Ok(field_id)
}

fn audit_field_definition(kind: AuditFieldKind, id: SymbolId) -> FieldDefinition {
    let options = FieldOptions {
        form_visible: false,
        form_editable: false,
        excel_import: false,
        ai_extract: false,
        filterable: matches!(kind, AuditFieldKind::TenantId | AuditFieldKind::Deleted),
        sortable: matches!(
            kind,
            AuditFieldKind::CreatedAt
                | AuditFieldKind::UpdatedAt
                | AuditFieldKind::DeletedAt
                | AuditFieldKind::Version
        ),
        ..FieldOptions::default()
    };
    FieldDefinition {
        id,
        name: kind.default_name().to_owned(),
        title: kind.default_title().to_owned(),
        value_type: kind.default_value_type(),
        state: DefinitionState::Known,
        required: false,
        options,
        relation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DefinitionState, GraphPatchBatch, PatchOrigin};

    #[test]
    fn enabling_and_disabling_audit_keeps_the_generated_field() -> anyhow::Result<()> {
        let mut model = ModelDefinition {
            id: SymbolId::new(),
            name: "work_order".to_owned(),
            title: "工单".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: Vec::new(),
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: ModelAuditDefinition::default(),
        };
        let mut program = crate::ProgramDefinition::empty("test", "测试");
        program.models.push(model.clone());

        let enabled = model_audit_patches(&model, AuditFieldKind::CreatedAt, true)
            .map_err(anyhow::Error::msg)?;
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            patches: enabled,
            origin: PatchOrigin::Studio,
        })?;
        model = program.models[0].clone();
        assert_eq!(model.fields.len(), 1);
        assert_eq!(model.audit.fields.len(), 1);

        let disabled = model_audit_patches(&model, AuditFieldKind::CreatedAt, false)
            .map_err(anyhow::Error::msg)?;
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            patches: disabled,
            origin: PatchOrigin::Studio,
        })?;
        assert_eq!(program.models[0].fields.len(), 1);
        assert!(program.models[0].audit.fields.is_empty());
        Ok(())
    }

    #[test]
    fn enabling_audit_rejects_a_reserved_field_with_the_wrong_type() {
        let kind = AuditFieldKind::CreatedAt;
        let model = ModelDefinition {
            id: SymbolId::new(),
            name: "work_order".to_owned(),
            title: "工单".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![FieldDefinition {
                id: SymbolId::new(),
                name: kind.default_name().to_owned(),
                title: kind.default_title().to_owned(),
                value_type: crate::ValueType::Text,
                state: DefinitionState::Known,
                required: false,
                options: FieldOptions::default(),
                relation: None,
            }],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: ModelAuditDefinition::default(),
        };

        let error = model_audit_patches(&model, kind, true).expect_err("类型不匹配必须失败");
        assert!(error.contains("created_at"));
    }
}
