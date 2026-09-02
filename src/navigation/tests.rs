use super::*;
use crate::{
    DefinitionState, EndpointInputDefinition, EndpointInputLocation, FieldDefinition, FieldOptions,
    FieldRelation, FunctionGraph, FunctionNode, FunctionNodeEditor, MenuRowActions,
    ModelAuditDefinition, ModelDefinition, PageDefinition, PageEndpointDefinition, PortDefinition,
    RelationKind, RestMethod, TableDefinition,
};

fn menu(page_id: Option<SymbolId>, children: Vec<MenuDefinition>) -> MenuDefinition {
    MenuDefinition {
        id: SymbolId::new(),
        name: "menu".to_owned(),
        title: "菜单".to_owned(),
        state: DefinitionState::Known,
        icon: None,
        page_id,
        enabled: true,
        children,
        required_permissions: Vec::new(),
        row_actions: MenuRowActions::default(),
    }
}

#[test]
fn studio_opens_the_application_workspace_by_default() {
    assert_eq!(StudioTab::default(), StudioTab::Applications);
}

#[test]
fn definition_search_matches_name_and_title() {
    assert!(definition_matches_search("work_order", "生产工单", "order"));
    assert!(definition_matches_search("work_order", "生产工单", "工单"));
    assert!(!definition_matches_search(
        "work_order",
        "生产工单",
        "device"
    ));
}

#[test]
fn draft_scene_selection_does_not_depend_on_a_published_scene() {
    let first = SymbolId::new();
    let second = SymbolId::new();
    let scenes = [first, second];

    assert_eq!(preferred_draft_scene_id(&scenes, None, None), Some(first));
    assert_eq!(
        preferred_draft_scene_id(&scenes, Some(first), Some(second)),
        Some(second)
    );
    assert_eq!(
        preferred_draft_scene_id(&scenes, Some(second), Some(SymbolId::new())),
        Some(second)
    );
}

#[test]
fn page_menu_references_include_nested_menus() {
    let page_id = SymbolId::new();
    let menus = vec![menu(
        Some(page_id),
        vec![menu(Some(page_id), Vec::new()), menu(None, Vec::new())],
    )];

    assert_eq!(page_menu_reference_count(&menus, page_id), 2);
}

#[test]
fn deleting_a_scene_keeps_pages_referenced_by_other_menus() {
    let target_scene_id = SymbolId::new();
    let retained_scene_id = SymbolId::new();
    let exclusive_page_id = SymbolId::new();
    let shared_page_id = SymbolId::new();
    let exclusive_route_id = SymbolId::new();
    let shared_route_id = SymbolId::new();
    let mut target_scene = menu(None, Vec::new());
    target_scene.id = target_scene_id;
    target_scene.children = vec![
        menu(Some(exclusive_page_id), Vec::new()),
        menu(Some(shared_page_id), Vec::new()),
    ];
    let mut retained_scene = menu(None, Vec::new());
    retained_scene.id = retained_scene_id;
    retained_scene.children = vec![menu(Some(shared_page_id), Vec::new())];
    let menus = vec![target_scene, retained_scene];
    let routes = vec![
        RouteDefinition {
            id: exclusive_route_id,
            name: "exclusive".to_owned(),
            path: "/exclusive".to_owned(),
            page_id: exclusive_page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        },
        RouteDefinition {
            id: shared_route_id,
            name: "shared".to_owned(),
            path: "/shared".to_owned(),
            page_id: shared_page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        },
    ];

    let deleted_ids = delete_menu_patches(&menus, &routes, target_scene_id)
        .into_iter()
        .map(|patch| match patch {
            GraphPatch::Delete { target_id } => target_id,
            _ => unreachable!("删除场景只应产生删除补丁"),
        })
        .collect::<BTreeSet<_>>();

    assert!(deleted_ids.contains(&target_scene_id));
    assert!(deleted_ids.contains(&exclusive_route_id));
    assert!(deleted_ids.contains(&exclusive_page_id));
    assert!(!deleted_ids.contains(&shared_route_id));
    assert!(!deleted_ids.contains(&shared_page_id));
}

#[test]
fn permission_usage_counts_every_supported_reference_owner() {
    let permission_id = SymbolId::new();
    let mut definition = ProgramDefinition::empty("permission-test", "权限测试");
    definition.permissions.push(crate::PermissionDefinition {
        id: permission_id,
        name: "asset:read".to_owned(),
        title: "查看资产".to_owned(),
        allowed_effects: Vec::new(),
    });

    let nested_menu = MenuDefinition {
        id: SymbolId::new(),
        name: "asset-list".to_owned(),
        title: "资产列表".to_owned(),
        state: DefinitionState::Known,
        icon: None,
        page_id: None,
        enabled: true,
        children: Vec::new(),
        required_permissions: vec![permission_id],
        row_actions: MenuRowActions::default(),
    };
    let root_actions = MenuRowActions {
        edit: crate::MenuActionAccess::Permission { permission_id },
        ..MenuRowActions::default()
    };
    definition.menus.push(MenuDefinition {
        id: SymbolId::new(),
        name: "assets".to_owned(),
        title: "资产".to_owned(),
        state: DefinitionState::Known,
        icon: None,
        page_id: None,
        enabled: true,
        children: vec![nested_menu],
        required_permissions: vec![permission_id],
        row_actions: root_actions,
    });
    definition.routes.push(RouteDefinition {
        id: SymbolId::new(),
        name: "assets".to_owned(),
        path: "/assets".to_owned(),
        page_id: SymbolId::new(),
        state: DefinitionState::Known,
        required_permissions: vec![permission_id],
    });
    definition.functions.push(FunctionDefinition {
        id: SymbolId::new(),
        name: "load_assets".to_owned(),
        title: "读取资产".to_owned(),
        state: DefinitionState::Known,
        inputs: Vec::new(),
        outputs: Vec::new(),
        graph: FunctionGraph::default(),
        required_permissions: vec![permission_id],
    });

    assert_eq!(
        permission_usage_map(&definition).get(&permission_id),
        Some(&5)
    );
}

#[test]
fn function_references_ignore_calls_owned_by_the_deleted_function() {
    let target_id = SymbolId::new();
    let caller_id = SymbolId::new();
    let mut definition = ProgramDefinition::empty("studio", "Studio");
    definition.functions.push(function(
        target_id,
        vec![FunctionNodeKind::ForEach {
            max_items: 10,
            body_function_id: target_id,
        }],
    ));
    definition.functions.push(function(
        caller_id,
        vec![FunctionNodeKind::ForEach {
            max_items: 10,
            body_function_id: target_id,
        }],
    ));

    assert_eq!(function_reference_count(&definition, target_id), 1);
    assert_eq!(function_reference_count(&definition, caller_id), 0);
}

#[test]
fn function_port_references_count_input_and_output_nodes() {
    let function_id = SymbolId::new();
    let port_id = SymbolId::new();
    let function = function(
        function_id,
        vec![
            FunctionNodeKind::Input { port_id },
            FunctionNodeKind::Output { port_id },
            FunctionNodeKind::Return,
        ],
    );

    assert_eq!(function_port_reference_count(&function, port_id), 2);
}

#[test]
fn function_node_references_count_structured_node_inputs() {
    let function_id = SymbolId::new();
    let referenced_node_id = SymbolId::new();
    let mut function = function(function_id, Vec::new());
    function.graph.nodes = vec![
        FunctionNode {
            id: referenced_node_id,
            name: "source".to_owned(),
            state: DefinitionState::Known,
            editor: FunctionNodeEditor::default(),
            kind: FunctionNodeKind::Constant {
                value: serde_json::json!("value"),
                value_type: ValueType::Text,
            },
        },
        FunctionNode {
            id: SymbolId::new(),
            name: "list".to_owned(),
            state: DefinitionState::Known,
            editor: FunctionNodeEditor::default(),
            kind: FunctionNodeKind::List {
                items: vec![referenced_node_id],
            },
        },
        FunctionNode {
            id: SymbolId::new(),
            name: "format".to_owned(),
            state: DefinitionState::Known,
            editor: FunctionNodeEditor::default(),
            kind: FunctionNodeKind::Format {
                template: "{}".to_owned(),
                values: vec![referenced_node_id],
            },
        },
    ];

    assert_eq!(
        function_node_reference_count(&function, referenced_node_id),
        2
    );
}

#[test]
fn model_usage_counts_distinct_external_reference_owners() {
    let target_model_id = SymbolId::new();
    let target_field_id = SymbolId::new();
    let source_model_id = SymbolId::new();
    let source_field_id = SymbolId::new();
    let mut definition = ProgramDefinition::empty("studio", "Studio");
    definition.models.push(model(
        target_model_id,
        "customer",
        vec![field(target_field_id, "name", ValueType::Text, None)],
    ));
    definition.models.push(model(
        source_model_id,
        "order",
        vec![field(
            source_field_id,
            "customer",
            ValueType::Optional {
                value: Box::new(ValueType::Object {
                    model_id: target_model_id,
                }),
            },
            Some(FieldRelation {
                kind: RelationKind::ManyToOne,
                target_model_id,
                target_field_id,
            }),
        )],
    ));
    definition.pages.push(PageDefinition {
        id: SymbolId::new(),
        name: "customer_list".to_owned(),
        title: "客户列表".to_owned(),
        state: DefinitionState::Known,
        renderer: PageRendererDefinition::CrudTable {
            table: TableDefinition {
                model_id: Some(target_model_id),
                page_size: 20,
            },
        },
        endpoints: vec![PageEndpointDefinition {
            id: SymbolId::new(),
            title: "导出客户".to_owned(),
            description: "导出客户数据".to_owned(),
            state: DefinitionState::Known,
            method: RestMethod::Post,
            path: "/api/customers/export".to_owned(),
            inputs: vec![EndpointInputDefinition {
                id: SymbolId::new(),
                name: "customer".to_owned(),
                title: "客户".to_owned(),
                location: EndpointInputLocation::Body,
                value_type: ValueType::Object {
                    model_id: target_model_id,
                },
                required: true,
            }],
            outputs: Vec::new(),
        }],
    });
    definition.functions.push(FunctionDefinition {
        id: SymbolId::new(),
        name: "load_customer".to_owned(),
        title: "读取客户".to_owned(),
        state: DefinitionState::Known,
        inputs: vec![PortDefinition {
            id: SymbolId::new(),
            name: "customer".to_owned(),
            value_type: ValueType::Object {
                model_id: target_model_id,
            },
        }],
        outputs: Vec::new(),
        graph: FunctionGraph {
            nodes: vec![FunctionNode {
                id: SymbolId::new(),
                name: "read_name".to_owned(),
                state: DefinitionState::Known,
                editor: FunctionNodeEditor::default(),
                kind: FunctionNodeKind::FieldAccess {
                    object: SymbolId::new(),
                    field_id: target_field_id,
                },
            }],
            edges: Vec::new(),
        },
        required_permissions: Vec::new(),
    });

    let usage = model_usage_summary(&definition, target_model_id);

    assert_eq!(
        usage,
        ModelUsageSummary {
            model_fields: 1,
            page_layouts: 1,
            page_endpoints: 1,
            functions: 1,
        }
    );
    assert_eq!(
        usage.description(),
        "1 个模型字段、1 个页面布局、1 个页面接口、1 个函数"
    );
}

#[test]
fn model_usage_ignores_references_owned_by_the_deleted_model() {
    let model_id = SymbolId::new();
    let mut definition = ProgramDefinition::empty("studio", "Studio");
    definition.models.push(model(
        model_id,
        "category",
        vec![field(
            SymbolId::new(),
            "parent",
            ValueType::Optional {
                value: Box::new(ValueType::Object { model_id }),
            },
            None,
        )],
    ));

    assert_eq!(
        model_usage_summary(&definition, model_id),
        ModelUsageSummary::default()
    );
}

#[test]
fn deleting_page_removes_its_routes_before_the_page() {
    let page_id = SymbolId::new();
    let route_id = SymbolId::new();
    let unrelated_route_id = SymbolId::new();
    let routes = vec![
        route(route_id, page_id),
        route(unrelated_route_id, SymbolId::new()),
    ];

    assert_eq!(
        delete_page_patches(&routes, page_id),
        vec![
            GraphPatch::Delete {
                target_id: route_id,
            },
            GraphPatch::Delete { target_id: page_id },
        ]
    );
}

fn route(id: SymbolId, page_id: SymbolId) -> RouteDefinition {
    RouteDefinition {
        id,
        name: "route".to_owned(),
        path: "/route".to_owned(),
        page_id,
        state: DefinitionState::Known,
        required_permissions: Vec::new(),
    }
}

fn model(id: SymbolId, name: &str, fields: Vec<FieldDefinition>) -> ModelDefinition {
    ModelDefinition {
        id,
        name: name.to_owned(),
        title: name.to_owned(),
        state: DefinitionState::Known,
        primary_key: crate::ModelPrimaryKeyDefinition::default(),
        fields,
        indexes: Vec::new(),
        queries: Vec::new(),
        validations: Vec::new(),
        audit: ModelAuditDefinition::default(),
    }
}

fn field(
    id: SymbolId,
    name: &str,
    value_type: ValueType,
    relation: Option<FieldRelation>,
) -> FieldDefinition {
    FieldDefinition {
        id,
        name: name.to_owned(),
        title: name.to_owned(),
        value_type,
        state: DefinitionState::Known,
        required: false,
        options: FieldOptions::default(),
        relation,
    }
}

fn function(id: SymbolId, kinds: Vec<FunctionNodeKind>) -> FunctionDefinition {
    FunctionDefinition {
        id,
        name: format!("function_{id}"),
        title: "函数".to_owned(),
        state: DefinitionState::Known,
        inputs: Vec::new(),
        outputs: Vec::new(),
        graph: FunctionGraph {
            nodes: kinds
                .into_iter()
                .enumerate()
                .map(|(index, kind)| FunctionNode {
                    id: SymbolId::new(),
                    name: format!("node_{index}"),
                    state: DefinitionState::Known,
                    editor: FunctionNodeEditor::default(),
                    kind,
                })
                .collect(),
            edges: Vec::new(),
        },
        required_permissions: Vec::new(),
    }
}
