use super::*;
use crate::{
    EffectKind, FunctionGraph, MenuActionAccess, MenuRowActions, PageEndpointDefinition,
    PageRendererDefinition, PermissionDefinition, RestMethod, TableDefinition,
};

#[test]
fn page_renderer_and_menu_actions_use_the_same_patch_protocol() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let page_id = SymbolId::new();
    let menu_id = SymbolId::new();
    let permission_id = SymbolId::new();
    program.pages.push(PageDefinition {
        id: page_id,
        name: "assets".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        renderer: PageRendererDefinition::ConventionFile,
        endpoints: Vec::new(),
    });
    program.menus.push(MenuDefinition {
        id: menu_id,
        name: "assets".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        icon: None,
        page_id: Some(page_id),
        enabled: true,
        children: Vec::new(),
        required_permissions: Vec::new(),
        row_actions: MenuRowActions::default(),
    });
    program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![
            GraphPatch::SetProperty {
                target_id: page_id,
                property: EditableProperty::PageRenderer,
                value: serde_json::to_value(PageRendererDefinition::CrudTable {
                    table: TableDefinition::default(),
                })?,
            },
            GraphPatch::SetProperty {
                target_id: menu_id,
                property: EditableProperty::MenuRowActions,
                value: serde_json::to_value(MenuRowActions {
                    detail: MenuActionAccess::Public,
                    edit: MenuActionAccess::Permission { permission_id },
                    delete: MenuActionAccess::Hidden,
                })?,
            },
        ],
    })?;
    assert!(matches!(
        program.pages[0].renderer,
        PageRendererDefinition::CrudTable { .. }
    ));
    assert_eq!(
        program.menus[0].row_actions.edit,
        MenuActionAccess::Permission { permission_id }
    );
    Ok(())
}

#[test]
fn permission_effects_use_property_patch() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let permission_id = SymbolId::new();
    program.permissions.push(PermissionDefinition {
        id: permission_id,
        name: "asset:write".to_owned(),
        title: "维护资产".to_owned(),
        allowed_effects: Vec::new(),
    });
    program.apply_patch(&GraphPatch::SetProperty {
        target_id: permission_id,
        property: EditableProperty::PermissionEffects,
        value: serde_json::json!([EffectKind::DatabaseRead, EffectKind::DatabaseWrite]),
    })?;
    assert_eq!(
        program.permissions[0].allowed_effects,
        vec![EffectKind::DatabaseRead, EffectKind::DatabaseWrite]
    );
    Ok(())
}

#[test]
fn route_permissions_use_property_patch() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let page_id = SymbolId::new();
    let route_id = SymbolId::new();
    let permission_id = SymbolId::new();
    program.pages.push(PageDefinition {
        id: page_id,
        name: "assets".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        renderer: PageRendererDefinition::ConventionFile,
        endpoints: Vec::new(),
    });
    program.routes.push(RouteDefinition {
        id: route_id,
        name: "assets".to_owned(),
        path: "/assets".to_owned(),
        page_id,
        state: crate::DefinitionState::Known,
        required_permissions: Vec::new(),
    });

    program.apply_patch(&GraphPatch::SetProperty {
        target_id: route_id,
        property: EditableProperty::RoutePermissions,
        value: serde_json::json!([permission_id]),
    })?;

    assert_eq!(program.routes[0].required_permissions, vec![permission_id]);
    Ok(())
}

#[test]
fn function_permissions_use_property_patch() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let function_id = SymbolId::new();
    let permission_id = SymbolId::new();
    program.functions.push(FunctionDefinition {
        id: function_id,
        name: "load_asset".to_owned(),
        title: "读取资产".to_owned(),
        state: crate::DefinitionState::Known,
        inputs: Vec::new(),
        outputs: Vec::new(),
        graph: FunctionGraph::default(),
        required_permissions: Vec::new(),
    });

    program.apply_patch(&GraphPatch::SetProperty {
        target_id: function_id,
        property: EditableProperty::FunctionPermissions,
        value: serde_json::json!([permission_id]),
    })?;

    assert_eq!(
        program.functions[0].required_permissions,
        vec![permission_id]
    );
    Ok(())
}

#[test]
fn function_ports_and_nodes_use_replacement_properties() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let function_id = SymbolId::new();
    let port_id = SymbolId::new();
    let node_id = SymbolId::new();
    program.functions.push(FunctionDefinition {
        id: function_id,
        name: "load_asset".to_owned(),
        title: "读取资产".to_owned(),
        state: crate::DefinitionState::Known,
        inputs: vec![PortDefinition {
            id: port_id,
            name: "asset_id".to_owned(),
            value_type: crate::ValueType::Text,
        }],
        outputs: Vec::new(),
        graph: FunctionGraph {
            nodes: vec![FunctionNode {
                id: node_id,
                name: "asset_id".to_owned(),
                state: crate::DefinitionState::Known,
                editor: crate::FunctionNodeEditor::default(),
                kind: crate::FunctionNodeKind::Input { port_id },
            }],
            edges: Vec::new(),
        },
        required_permissions: Vec::new(),
    });
    let updated_port = PortDefinition {
        id: port_id,
        name: "id".to_owned(),
        value_type: crate::ValueType::Integer,
    };
    let updated_node = FunctionNode {
        id: node_id,
        name: "return".to_owned(),
        state: crate::DefinitionState::Known,
        editor: crate::FunctionNodeEditor { x: 120, y: 80 },
        kind: crate::FunctionNodeKind::Return,
    };

    program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![
            GraphPatch::SetProperty {
                target_id: port_id,
                property: EditableProperty::FunctionPort,
                value: serde_json::to_value(updated_port.clone())?,
            },
            GraphPatch::SetProperty {
                target_id: node_id,
                property: EditableProperty::FunctionNode,
                value: serde_json::to_value(updated_node.clone())?,
            },
        ],
    })?;

    assert_eq!(program.functions[0].inputs, vec![updated_port]);
    assert_eq!(program.functions[0].graph.nodes, vec![updated_node]);
    Ok(())
}

#[test]
fn function_graph_position_and_edges_use_graph_patches() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let function_id = SymbolId::new();
    let source_node_id = SymbolId::new();
    let target_node_id = SymbolId::new();
    let edge_id = SymbolId::new();
    let node = |id, name: &str, kind| FunctionNode {
        id,
        name: name.to_owned(),
        state: crate::DefinitionState::Known,
        editor: crate::FunctionNodeEditor::default(),
        kind,
    };
    program.functions.push(FunctionDefinition {
        id: function_id,
        name: "load_asset".to_owned(),
        title: "读取资产".to_owned(),
        state: crate::DefinitionState::Known,
        inputs: Vec::new(),
        outputs: Vec::new(),
        graph: FunctionGraph {
            nodes: vec![
                node(
                    source_node_id,
                    "source",
                    crate::FunctionNodeKind::Constant {
                        value: serde_json::json!("asset"),
                        value_type: crate::ValueType::Text,
                    },
                ),
                node(target_node_id, "target", crate::FunctionNodeKind::Return),
            ],
            edges: Vec::new(),
        },
        required_permissions: Vec::new(),
    });
    let position = crate::FunctionNodeEditor { x: 320, y: 180 };
    let edge = GraphEdge {
        id: edge_id,
        from_node: source_node_id,
        from_port: "out".to_owned(),
        to_node: target_node_id,
        to_port: "in".to_owned(),
    };

    program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![
            GraphPatch::SetProperty {
                target_id: source_node_id,
                property: EditableProperty::FunctionNodePosition,
                value: serde_json::to_value(position)?,
            },
            GraphPatch::Connect {
                function_id,
                edge: edge.clone(),
            },
        ],
    })?;

    assert_eq!(program.functions[0].graph.nodes[0].editor, position);
    assert_eq!(program.functions[0].graph.edges, vec![edge]);

    program.apply_patch(&GraphPatch::Disconnect {
        function_id,
        edge_id,
    })?;
    assert!(program.functions[0].graph.edges.is_empty());
    Ok(())
}

#[test]
fn function_graph_rejects_edges_into_constant_nodes() {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let function_id = SymbolId::new();
    let boolean_id = SymbolId::new();
    let constant_id = SymbolId::new();
    program.functions.push(FunctionDefinition {
        id: function_id,
        name: "check_asset".to_owned(),
        title: "检查资产".to_owned(),
        state: crate::DefinitionState::Known,
        inputs: Vec::new(),
        outputs: Vec::new(),
        graph: FunctionGraph {
            nodes: vec![
                FunctionNode {
                    id: boolean_id,
                    name: "boolean".to_owned(),
                    state: crate::DefinitionState::Known,
                    editor: crate::FunctionNodeEditor::default(),
                    kind: crate::FunctionNodeKind::Boolean {
                        operator: crate::BooleanOperator::And,
                    },
                },
                FunctionNode {
                    id: constant_id,
                    name: "constant".to_owned(),
                    state: crate::DefinitionState::Known,
                    editor: crate::FunctionNodeEditor::default(),
                    kind: crate::FunctionNodeKind::Constant {
                        value: serde_json::json!("value"),
                        value_type: crate::ValueType::Text,
                    },
                },
            ],
            edges: Vec::new(),
        },
        required_permissions: Vec::new(),
    });

    let error = program
        .apply_patch(&GraphPatch::Connect {
            function_id,
            edge: GraphEdge {
                id: SymbolId::new(),
                from_node: boolean_id,
                from_port: "out".to_owned(),
                to_node: constant_id,
                to_port: "in".to_owned(),
            },
        })
        .err();

    assert!(error.is_some_and(|error| error.to_string().contains("不能连接")));
    assert!(program.functions[0].graph.edges.is_empty());
}

#[test]
fn patch_batch_is_atomic_on_invalid_target() {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let original = program.clone();
    let result = program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![GraphPatch::Rename {
            target_id: SymbolId::new(),
            name: "missing".to_owned(),
            title: None,
        }],
    });
    assert!(result.is_err());
    assert_eq!(program, original);
}

#[test]
fn page_endpoint_uses_page_owned_patch_collection() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let page_id = SymbolId::new();
    let endpoint_id = SymbolId::new();
    program.pages.push(PageDefinition {
        id: page_id,
        name: "assets".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        renderer: PageRendererDefinition::ConventionFile,
        endpoints: Vec::new(),
    });
    let endpoint = PageEndpointDefinition {
        id: endpoint_id,
        title: "归档资产".to_owned(),
        description: "归档指定资产".to_owned(),
        state: crate::DefinitionState::Known,
        method: RestMethod::Post,
        path: "/api/assets/archive".to_owned(),
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    program.apply_patch(&GraphPatch::Insert {
        parent_id: page_id,
        collection: ChildCollection::PageEndpoints,
        index: 0,
        entity: Box::new(GraphEntity::PageEndpoint(endpoint.clone())),
    })?;
    assert_eq!(program.pages[0].endpoints, vec![endpoint.clone()]);

    let mut updated = endpoint;
    updated.method = RestMethod::Delete;
    program.apply_patch(&GraphPatch::SetProperty {
        target_id: endpoint_id,
        property: EditableProperty::PageEndpoint,
        value: serde_json::to_value(updated.clone())?,
    })?;
    assert_eq!(program.pages[0].endpoints, vec![updated]);
    Ok(())
}

#[test]
fn model_grid_properties_update_fields_and_indexes() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let model_id = SymbolId::new();
    let first_field_id = SymbolId::new();
    let second_field_id = SymbolId::new();
    let index_id = SymbolId::new();
    program.models.push(ModelDefinition {
        id: model_id,
        name: "asset".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        primary_key: crate::ModelPrimaryKeyDefinition::default(),
        fields: vec![
            FieldDefinition {
                id: first_field_id,
                name: "name".to_owned(),
                title: "名称".to_owned(),
                value_type: crate::ValueType::Text,
                state: crate::DefinitionState::Known,
                required: false,
                options: crate::FieldOptions::default(),
                relation: None,
            },
            FieldDefinition {
                id: second_field_id,
                name: "count".to_owned(),
                title: "数量".to_owned(),
                value_type: crate::ValueType::Integer,
                state: crate::DefinitionState::Known,
                required: false,
                options: crate::FieldOptions::default(),
                relation: None,
            },
        ],
        indexes: vec![ModelIndexDefinition {
            id: index_id,
            fields: vec![first_field_id],
            unique: false,
        }],
        queries: Vec::new(),
        validations: Vec::new(),
        audit: crate::ModelAuditDefinition::default(),
    });

    program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldValueType,
                value: serde_json::to_value(crate::ValueType::Boolean)?,
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldOptions,
                value: serde_json::to_value(crate::FieldOptions {
                    list_visible: false,
                    unique: true,
                    validation: crate::FieldValidation {
                        min_length: Some(2),
                        ..crate::FieldValidation::default()
                    },
                    ..crate::FieldOptions::default()
                })?,
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldRequired,
                value: serde_json::json!(true),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldListVisible,
                value: serde_json::json!(false),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldDetailVisible,
                value: serde_json::json!(false),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldFormVisible,
                value: serde_json::json!(false),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldFormEditable,
                value: serde_json::json!(false),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldFilterable,
                value: serde_json::json!(true),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldSortable,
                value: serde_json::json!(true),
            },
            GraphPatch::SetProperty {
                target_id: first_field_id,
                property: EditableProperty::FieldUnique,
                value: serde_json::json!(false),
            },
            GraphPatch::SetProperty {
                target_id: index_id,
                property: EditableProperty::ModelIndexFields,
                value: serde_json::to_value(vec![first_field_id, second_field_id])?,
            },
            GraphPatch::SetProperty {
                target_id: index_id,
                property: EditableProperty::ModelIndexUnique,
                value: serde_json::json!(true),
            },
            GraphPatch::SetProperty {
                target_id: model_id,
                property: EditableProperty::ModelPrimaryKey,
                value: serde_json::json!(crate::ModelPrimaryKeyDefinition {
                    generation: crate::PrimaryKeyGeneration::AutoIncrement,
                }),
            },
            GraphPatch::SetProperty {
                target_id: model_id,
                property: EditableProperty::ModelAudit,
                value: serde_json::json!(crate::ModelAuditDefinition {
                    fields: vec![crate::ModelAuditField {
                        kind: crate::AuditFieldKind::Version,
                        field_id: second_field_id,
                    }],
                }),
            },
        ],
    })?;

    assert_eq!(
        program.models[0].fields[0].value_type,
        crate::ValueType::Boolean
    );
    assert!(program.models[0].fields[0].required);
    assert!(!program.models[0].fields[0].options.list_visible);
    assert!(!program.models[0].fields[0].options.detail_visible);
    assert!(!program.models[0].fields[0].options.form_visible);
    assert!(!program.models[0].fields[0].options.form_editable);
    assert!(program.models[0].fields[0].options.filterable);
    assert!(program.models[0].fields[0].options.sortable);
    assert!(!program.models[0].fields[0].options.unique);
    assert_eq!(
        program.models[0].fields[0].options.validation.min_length,
        Some(2)
    );
    assert_eq!(
        program.models[0].indexes[0].fields,
        vec![first_field_id, second_field_id]
    );
    assert!(program.models[0].indexes[0].unique);
    assert_eq!(
        program.models[0].primary_key.generation,
        crate::PrimaryKeyGeneration::AutoIncrement
    );
    assert_eq!(
        program.models[0].audit.fields[0].kind,
        crate::AuditFieldKind::Version
    );
    Ok(())
}

#[test]
fn system_id_cannot_be_inserted_as_a_regular_field() {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let model_id = SymbolId::new();
    program.models.push(ModelDefinition {
        id: model_id,
        name: "asset".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        primary_key: crate::ModelPrimaryKeyDefinition::default(),
        fields: Vec::new(),
        indexes: Vec::new(),
        queries: Vec::new(),
        validations: Vec::new(),
        audit: crate::ModelAuditDefinition::default(),
    });

    let error = program
        .apply_patch(&GraphPatch::Insert {
            parent_id: model_id,
            collection: ChildCollection::Fields,
            index: 0,
            entity: Box::new(GraphEntity::Field(FieldDefinition {
                id: SymbolId::new(),
                name: "id".to_owned(),
                title: "ID".to_owned(),
                value_type: crate::ValueType::Text,
                state: crate::DefinitionState::Known,
                required: true,
                options: crate::FieldOptions::default(),
                relation: None,
            })),
        })
        .expect_err("系统 id 不能作为普通字段插入");
    assert!(error.to_string().contains("系统主键字段"));
}

#[test]
fn model_designer_updates_structured_query_validation_and_relation() -> anyhow::Result<()> {
    let mut program = ProgramDefinition::empty("inventory", "资产");
    let model_id = SymbolId::new();
    let source_field_id = SymbolId::new();
    let target_field_id = SymbolId::new();
    let query_id = SymbolId::new();
    let validation_id = SymbolId::new();
    program.models.push(ModelDefinition {
        id: model_id,
        name: "asset".to_owned(),
        title: "资产".to_owned(),
        state: crate::DefinitionState::Known,
        primary_key: crate::ModelPrimaryKeyDefinition::default(),
        fields: vec![
            FieldDefinition {
                id: source_field_id,
                name: "owner".to_owned(),
                title: "负责人".to_owned(),
                value_type: crate::ValueType::Text,
                state: crate::DefinitionState::Known,
                required: false,
                options: crate::FieldOptions::default(),
                relation: None,
            },
            FieldDefinition {
                id: target_field_id,
                name: "owner_name".to_owned(),
                title: "负责人名称".to_owned(),
                value_type: crate::ValueType::Text,
                state: crate::DefinitionState::Known,
                required: false,
                options: crate::FieldOptions::default(),
                relation: None,
            },
        ],
        indexes: Vec::new(),
        queries: vec![crate::ModelQueryDefinition {
            id: query_id,
            name: "by_owner".to_owned(),
            title: "按负责人".to_owned(),
            conjunction: crate::QueryConjunction::All,
            conditions: Vec::new(),
        }],
        validations: vec![crate::ModelValidationDefinition {
            id: validation_id,
            message: "负责人不能为空".to_owned(),
            rule: crate::ModelValidationRule::RequiredWhenPresent {
                field_id: source_field_id,
                when_field_id: target_field_id,
            },
        }],
        audit: crate::ModelAuditDefinition::default(),
    });

    let relation = crate::FieldRelation {
        kind: crate::RelationKind::ManyToOne,
        target_model_id: model_id,
        target_field_id,
    };
    let updated_query = crate::ModelQueryDefinition {
        id: query_id,
        name: "by_owner_name".to_owned(),
        title: "按负责人名称".to_owned(),
        conjunction: crate::QueryConjunction::Any,
        conditions: vec![crate::QueryCondition::Field {
            field_id: target_field_id,
            operator: crate::QueryOperator::Contains,
            parameter: "owner_name".to_owned(),
        }],
    };
    let updated_validation = crate::ModelValidationDefinition {
        id: validation_id,
        message: "负责人字段必须一起填写".to_owned(),
        rule: crate::ModelValidationRule::FieldsRequiredTogether {
            field_ids: vec![source_field_id, target_field_id],
        },
    };
    program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![
            GraphPatch::SetProperty {
                target_id: source_field_id,
                property: EditableProperty::FieldRelation,
                value: serde_json::to_value(relation.clone())?,
            },
            GraphPatch::SetProperty {
                target_id: source_field_id,
                property: EditableProperty::FieldValueType,
                value: serde_json::to_value(crate::ValueType::Object { model_id })?,
            },
            GraphPatch::SetProperty {
                target_id: query_id,
                property: EditableProperty::ModelQuery,
                value: serde_json::to_value(updated_query.clone())?,
            },
            GraphPatch::SetProperty {
                target_id: validation_id,
                property: EditableProperty::ModelValidation,
                value: serde_json::to_value(updated_validation.clone())?,
            },
        ],
    })?;

    assert_eq!(program.models[0].fields[0].relation, Some(relation));
    assert_eq!(program.models[0].queries, vec![updated_query]);
    assert_eq!(program.models[0].validations, vec![updated_validation]);

    program.apply_patch_batch(&GraphPatchBatch {
        base_version: 0,
        origin: PatchOrigin::Studio,
        patches: vec![
            GraphPatch::Delete {
                target_id: query_id,
            },
            GraphPatch::Delete {
                target_id: validation_id,
            },
        ],
    })?;
    assert!(program.models[0].queries.is_empty());
    assert!(program.models[0].validations.is_empty());
    Ok(())
}
