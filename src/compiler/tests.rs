    use super::*;
    use crate::{
        DefinitionState, EndpointInputDefinition, EndpointInputLocation, EndpointOutputDefinition,
        FieldDefinition, FunctionGraph, FunctionNodeEditor, MenuActionAccess, MenuDefinition,
        MenuRowActions, ModelDefinition, PageEndpointDefinition, PageRendererDefinition,
        PermissionDefinition, PortDefinition, RestMethod, RouteDefinition, TableDefinition,
        TreeDefinition, ValueType,
    };

    #[test]
    fn compile_failure_display_includes_actionable_diagnostics() {
        let symbol_id = SymbolId::new();
        let failure = CompileFailure {
            diagnostics: vec![diagnostic(
                "PAGE_ENDPOINT_INPUT_INVALID",
                CompilerStage::Schema,
                "接口入参标识无效",
                Some(symbol_id),
            )],
        };

        let message = failure.to_string();
        assert!(message.contains("PAGE_ENDPOINT_INPUT_INVALID"));
        assert!(message.contains("接口入参标识无效"));
        assert!(message.contains(&symbol_id.to_string()));
    }

    fn model(name: &str, title: &str) -> ModelDefinition {
        ModelDefinition {
            id: SymbolId::new(),
            name: name.to_owned(),
            title: title.to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: SymbolId::new(),
                    name: "name".to_owned(),
                    title: "名称".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions {
                        filterable: true,
                        ..crate::FieldOptions::default()
                    },
                    relation: None,
                },
                FieldDefinition {
                    id: SymbolId::new(),
                    name: "category_id".to_owned(),
                    title: "分类".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
        }
    }

    fn crud_program() -> ProgramDefinition {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let model = model("asset", "资产");
        let page_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "asset_list".to_owned(),
            title: "资产列表".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::CrudTable {
                table: TableDefinition {
                    model_id: Some(model.id),
                    page_size: 20,
                },
            },
            endpoints: Vec::new(),
        });
        program.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: "asset_list".to_owned(),
            path: "/assets".to_owned(),
            page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });
        program.menus.push(MenuDefinition {
            id: SymbolId::new(),
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            icon: None,
            page_id: Some(page_id),
            enabled: true,
            children: Vec::new(),
            required_permissions: Vec::new(),
            row_actions: MenuRowActions {
                detail: MenuActionAccess::Public,
                edit: MenuActionAccess::Public,
                delete: MenuActionAccess::Hidden,
            },
        });
        program.models.push(model);
        program
    }

    #[test]
    fn compiles_crud_page_from_model_metadata() -> anyhow::Result<()> {
        let program = crud_program();
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        let page = &image.pages[&program.pages[0].id];
        assert!(matches!(
            page.renderer,
            CompiledPageRenderer::CrudTable { .. }
        ));
        assert_eq!(page.endpoints.len(), 6);
        let model_id = program.models[0].id;
        assert_eq!(
            page.endpoints[0].path,
            format!("/api/runtime/models/{model_id}/records")
        );
        assert_eq!(
            page.endpoints[4].path,
            format!("/api/runtime/models/{model_id}/records/import")
        );
        assert_eq!(image.models[&program.models[0].id].title, "资产");
        Ok(())
    }

    #[test]
    fn compiles_program_menu_tree_without_record_endpoints() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("admin", "管理后台");
        let page_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "menus".to_owned(),
            title: "菜单挂载".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::MenuTree,
            endpoints: Vec::new(),
        });
        program.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: "menus".to_owned(),
            path: "/menus".to_owned(),
            page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });

        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-menu",
            ImageTarget::Universal,
        )?;
        let page = &image.pages[&page_id];

        assert!(matches!(&page.renderer, CompiledPageRenderer::MenuTree));
        assert!(page.endpoints.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_invalid_or_duplicate_permission_identifiers() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.permissions = vec![
            PermissionDefinition {
                id: SymbolId::new(),
                name: "asset.read".to_owned(),
                title: "查看资产".to_owned(),
                allowed_effects: Vec::new(),
            },
            PermissionDefinition {
                id: SymbolId::new(),
                name: "asset.read".to_owned(),
                title: "重复权限".to_owned(),
                allowed_effects: Vec::new(),
            },
        ];
        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无效权限定义不应通过编译"))?;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PERMISSION_IDENTIFIER_INVALID")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PERMISSION_IDENTIFIER_DUPLICATE")
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_or_duplicate_page_identifiers() -> anyhow::Result<()> {
        let mut program = crud_program();
        let mut duplicate = program.pages[0].clone();
        duplicate.id = SymbolId::new();
        let mut invalid = program.pages[0].clone();
        invalid.id = SymbolId::new();
        invalid.name = "Asset Page".to_owned();
        invalid.title.clear();
        program.pages.extend([duplicate, invalid]);

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无效页面定义不应通过编译"))?;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_IDENTIFIER_INVALID")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_IDENTIFIER_DUPLICATE")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_TITLE_EMPTY")
        );
        Ok(())
    }

    #[test]
    fn rejects_page_without_route() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.routes.clear();

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无路由页面不应通过编译"))?;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_ROUTE_MISSING")
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_duplicate_or_untitled_functions() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.functions = vec![
            FunctionDefinition {
                id: SymbolId::new(),
                name: "Load Asset".to_owned(),
                title: String::new(),
                state: DefinitionState::Known,
                inputs: Vec::new(),
                outputs: Vec::new(),
                graph: FunctionGraph::default(),
                required_permissions: Vec::new(),
            },
            FunctionDefinition {
                id: SymbolId::new(),
                name: "Load Asset".to_owned(),
                title: "重复函数".to_owned(),
                state: DefinitionState::Known,
                inputs: Vec::new(),
                outputs: Vec::new(),
                graph: FunctionGraph::default(),
                required_permissions: Vec::new(),
            },
        ];

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无效函数定义不应通过编译"))?;

        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "FUNCTION_IDENTIFIER_INVALID")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "FUNCTION_TITLE_EMPTY")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "FUNCTION_IDENTIFIER_DUPLICATE")
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_model_and_field_references_in_structured_types() -> anyhow::Result<()> {
        let mut program = crud_program();
        let missing_model_id = SymbolId::new();
        let missing_field_id = SymbolId::new();
        let field_owner_id = program.models[0].fields[0].id;
        program.models[0].fields[0].value_type = ValueType::Optional {
            value: Box::new(ValueType::Object {
                model_id: missing_model_id,
            }),
        };
        let port_id = SymbolId::new();
        let node_id = SymbolId::new();
        let input_node_id = SymbolId::new();
        let object_node_id = SymbolId::new();
        let list_node_id = SymbolId::new();
        let format_node_id = SymbolId::new();
        let validation_node_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: SymbolId::new(),
            name: "load_asset".to_owned(),
            title: "读取资产".to_owned(),
            state: DefinitionState::Known,
            inputs: vec![PortDefinition {
                id: port_id,
                name: "asset".to_owned(),
                value_type: ValueType::Object {
                    model_id: missing_model_id,
                },
            }],
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![
                    FunctionNode {
                        id: node_id,
                        name: "read_missing_field".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::FieldAccess {
                            object: SymbolId::new(),
                            field_id: missing_field_id,
                        },
                    },
                    FunctionNode {
                        id: input_node_id,
                        name: "missing_input".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Input {
                            port_id: SymbolId::new(),
                        },
                    },
                    FunctionNode {
                        id: object_node_id,
                        name: "object_with_missing_references".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Object {
                            fields: BTreeMap::from([(missing_field_id, SymbolId::new())]),
                        },
                    },
                    FunctionNode {
                        id: list_node_id,
                        name: "list_with_missing_item".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::List {
                            items: vec![SymbolId::new()],
                        },
                    },
                    FunctionNode {
                        id: format_node_id,
                        name: "format_with_missing_value".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Format {
                            template: "{0}".to_owned(),
                            values: vec![SymbolId::new()],
                        },
                    },
                    FunctionNode {
                        id: validation_node_id,
                        name: "validation_with_missing_field".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::ValidateForm {
                            rules: vec![crate::ValidationRule {
                                field_id: missing_field_id,
                                rule: crate::ValidationRuleKind::Required,
                                message: "不能为空".to_owned(),
                            }],
                        },
                    },
                ],
                edges: Vec::new(),
            },
            required_permissions: Vec::new(),
        });

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("悬空模型与字段引用不应通过编译"))?;
        let unresolved_owners = failure
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SYMBOL_UNRESOLVED")
            .filter_map(|diagnostic| diagnostic.symbol_id)
            .collect::<BTreeSet<_>>();

        assert!(unresolved_owners.contains(&field_owner_id));
        assert!(unresolved_owners.contains(&port_id));
        assert!(unresolved_owners.contains(&node_id));
        assert!(unresolved_owners.contains(&input_node_id));
        assert!(unresolved_owners.contains(&object_node_id));
        assert!(unresolved_owners.contains(&list_node_id));
        assert!(unresolved_owners.contains(&format_node_id));
        assert!(unresolved_owners.contains(&validation_node_id));
        Ok(())
    }

    #[test]
    fn compiles_composable_model_audit_metadata() -> anyhow::Result<()> {
        let tenant_id = SymbolId::new();
        let deleted_id = SymbolId::new();
        let model_id = SymbolId::new();
        let model = ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: tenant_id,
                    name: "tenant_id".to_owned(),
                    title: "租户".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: deleted_id,
                    name: "deleted".to_owned(),
                    title: "逻辑删除".to_owned(),
                    value_type: ValueType::Boolean,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition {
                fields: vec![
                    crate::ModelAuditField {
                        kind: crate::AuditFieldKind::TenantId,
                        field_id: tenant_id,
                    },
                    crate::ModelAuditField {
                        kind: crate::AuditFieldKind::Deleted,
                        field_id: deleted_id,
                    },
                ],
            },
        };
        let mut program = ProgramDefinition::empty("inventory", "资产");
        program.models.push(model);
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-audit",
            ImageTarget::Universal,
        )?;
        assert_eq!(image.models[&model_id].audit.fields.len(), 2);
        Ok(())
    }

    #[test]
    fn compiles_bidirectional_relationship_query_and_validation_metadata() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("directory", "组织目录");
        let department_id = SymbolId::new();
        let department_name_id = SymbolId::new();
        let department_users_id = SymbolId::new();
        let user_id = SymbolId::new();
        let user_name_id = SymbolId::new();
        let user_department_id = SymbolId::new();
        let user_phone_id = SymbolId::new();
        let user_email_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: department_id,
            name: "department".to_owned(),
            title: "部门".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: department_name_id,
                    name: "name".to_owned(),
                    title: "部门名称".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: department_users_id,
                    name: "users".to_owned(),
                    title: "用户".to_owned(),
                    value_type: ValueType::List {
                        item: Box::new(ValueType::Object { model_id: user_id }),
                    },
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions {
                        validation: crate::FieldValidation {
                            unique_items: true,
                            ..crate::FieldValidation::default()
                        },
                        ..crate::FieldOptions::default()
                    },
                    relation: Some(crate::FieldRelation {
                        kind: crate::RelationKind::OneToMany,
                        target_model_id: user_id,
                        target_field_id: user_department_id,
                    }),
                },
            ],
            indexes: vec![crate::ModelIndexDefinition {
                id: SymbolId::new(),
                fields: vec![department_name_id],
                unique: false,
            }],
            queries: vec![crate::ModelQueryDefinition {
                id: SymbolId::new(),
                name: "search_by_name".to_owned(),
                title: "按部门和用户名查询".to_owned(),
                conjunction: crate::QueryConjunction::All,
                conditions: vec![
                    crate::QueryCondition::Field {
                        field_id: department_name_id,
                        operator: crate::QueryOperator::Contains,
                        parameter: "department_name".to_owned(),
                    },
                    crate::QueryCondition::Relation {
                        relation_field_id: department_users_id,
                        target_field_id: user_name_id,
                        operator: crate::QueryOperator::Contains,
                        parameter: "user_name".to_owned(),
                    },
                ],
            }],
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
        });
        program.models.push(ModelDefinition {
            id: user_id,
            name: "user".to_owned(),
            title: "用户".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: user_name_id,
                    name: "name".to_owned(),
                    title: "用户名".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: user_department_id,
                    name: "department".to_owned(),
                    title: "部门".to_owned(),
                    value_type: ValueType::Object {
                        model_id: department_id,
                    },
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: Some(crate::FieldRelation {
                        kind: crate::RelationKind::ManyToOne,
                        target_model_id: department_id,
                        target_field_id: department_users_id,
                    }),
                },
                FieldDefinition {
                    id: user_phone_id,
                    name: "phone".to_owned(),
                    title: "手机号".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: user_email_id,
                    name: "email".to_owned(),
                    title: "邮箱".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: vec![crate::ModelValidationDefinition {
                id: SymbolId::new(),
                message: "填写手机号时必须填写邮箱".to_owned(),
                rule: crate::ModelValidationRule::RequiredWhenPresent {
                    field_id: user_email_id,
                    when_field_id: user_phone_id,
                },
            }],
            audit: crate::ModelAuditDefinition::default(),
        });
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-relationship",
            ImageTarget::Universal,
        )?;
        assert_eq!(image.models.len(), 2);
        let user_model = &image.models[&user_id];
        let department_slot = user_model.field_slots[&user_department_id];
        assert_eq!(
            user_model.field_relations[&department_slot].kind,
            crate::RelationKind::ManyToOne
        );
        assert_eq!(
            user_model.field_relations[&department_slot].target_model_id,
            department_id
        );
        Ok(())
    }

    #[test]
    fn compiles_custom_rest_endpoint_from_metadata() -> anyhow::Result<()> {
        let mut program = crud_program();
        let endpoint_id = SymbolId::new();
        program.pages[0].endpoints.push(PageEndpointDefinition {
            id: endpoint_id,
            title: "批量停用资产".to_owned(),
            description: "批量停用指定分类中的资产".to_owned(),
            state: DefinitionState::Known,
            method: RestMethod::Post,
            path: "/api/assets/{categoryId}/batch-disable".to_owned(),
            inputs: vec![EndpointInputDefinition {
                id: SymbolId::new(),
                name: "categoryId".to_owned(),
                title: "分类 ID".to_owned(),
                location: EndpointInputLocation::Path,
                value_type: ValueType::Text,
                required: true,
            }],
            outputs: vec![EndpointOutputDefinition {
                id: SymbolId::new(),
                name: "updatedCount".to_owned(),
                title: "停用数量".to_owned(),
                value_type: ValueType::Integer,
            }],
        });

        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        let endpoint = image.pages[&program.pages[0].id]
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id.to_string())
            .ok_or_else(|| anyhow::anyhow!("自定义接口未进入编译产物"))?;
        assert_eq!(endpoint.method, RestMethod::Post);
        assert_eq!(endpoint.title, "批量停用资产");
        assert_eq!(endpoint.source, PageEndpointSource::Convention);
        Ok(())
    }

    #[test]
    fn rejects_declared_route_that_conflicts_with_builtin_endpoint() {
        let mut program = crud_program();
        let model_id = program.models[0].id;
        program.pages[0].endpoints.push(PageEndpointDefinition {
            id: SymbolId::new(),
            title: "重复查询".to_owned(),
            description: "错误覆盖内置查询".to_owned(),
            state: DefinitionState::Known,
            method: RestMethod::Get,
            path: format!("/api/runtime/models/{model_id}/records"),
            inputs: Vec::new(),
            outputs: Vec::new(),
        });

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err();

        assert!(failure.is_some_and(|failure| {
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_ENDPOINT_ROUTE_DUPLICATE_GLOBAL")
        }));
    }

    #[test]
    fn allows_multiple_pages_to_consume_the_same_builtin_model_routes() -> anyhow::Result<()> {
        let mut program = crud_program();
        let mut second_page = program.pages[0].clone();
        second_page.id = SymbolId::new();
        second_page.name = "asset_overview".to_owned();
        second_page.title = "资产概览".to_owned();
        program.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: second_page.name.clone(),
            path: "/asset-overview".to_owned(),
            page_id: second_page.id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });
        program.pages.push(second_page);

        ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        Ok(())
    }

    #[test]
    fn rejects_builtin_page_without_model() {
        let mut program = crud_program();
        program.pages[0].renderer = PageRendererDefinition::CrudTable {
            table: TableDefinition::default(),
        };
        let error = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .expect_err("缺少模型必须被拒绝");
        assert!(
            error
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "TABLE_MODEL_REQUIRED")
        );
    }

    #[test]
    fn compiles_tree_table_with_explicit_relation() -> anyhow::Result<()> {
        let mut program = crud_program();
        let tree_model = model("category", "分类");
        let table_model = program.models[0].clone();
        program.pages[0].renderer = PageRendererDefinition::TreeTable {
            tree: TreeDefinition {
                model_id: Some(tree_model.id),
                label_field_id: Some(tree_model.fields[0].id),
                parent_field_id: Some(tree_model.fields[1].id),
                table_relation_field_id: Some(table_model.fields[1].id),
            },
            table: TableDefinition {
                model_id: Some(table_model.id),
                page_size: 20,
            },
        };
        program.models.push(tree_model);
        ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        Ok(())
    }

