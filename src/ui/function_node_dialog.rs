use super::*;

#[component]
pub(super) fn FunctionNodeDialog(
    node: Option<FunctionNode>,
    function: FunctionDefinition,
    models: Vec<ModelDefinition>,
    routes: Vec<RouteDefinition>,
    functions: Vec<FunctionDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let editing = node.is_some();
    let node_count = function.graph.nodes.len();
    let next_editor = next_function_node_editor(&function.graph.nodes);
    let initial_node = node.unwrap_or_else(|| FunctionNode {
        id: SymbolId::new(),
        name: next_function_node_name(&function.graph.nodes),
        state: DefinitionState::Known,
        editor: next_editor,
        kind: FunctionNodeKind::Constant {
            value: serde_json::Value::String(String::new()),
            value_type: ValueType::Text,
        },
    });
    let node_id = initial_node.id;
    let mut draft = use_signal(move || initial_node);
    let current = draft();
    let current_kind_key = function_node_kind_key(&current.kind);
    let catalog_api = api_base_url.clone();
    let capability_catalog = use_resource(move || {
        let api_base_url = catalog_api.clone();
        async move { get_api::<crate::StudioCatalog>(&api_base_url, "/api/studio/catalog").await }
    });
    let capabilities = capability_catalog
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|catalog| catalog.capabilities.clone())
        .unwrap_or_default();
    let option_function = function.clone();
    let option_models = models.clone();
    let option_routes = routes.clone();
    let option_functions = functions.clone();
    let option_capabilities = capabilities.clone();

    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-function-node-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑函数节点" } else { "新建函数节点" } }
                    DialogDescription { "{function.title} · {function.name}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭节点编辑",
                    aria_label: "关闭节点编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let node = draft();
                if node.name.trim().is_empty() {
                    status.set(Some("节点名称不能为空".to_owned()));
                    return;
                }
                let patch = if editing {
                    GraphPatch::SetProperty {
                        target_id: node_id,
                        property: crate::EditableProperty::FunctionNode,
                        value: serde_json::json!(node),
                    }
                } else {
                    GraphPatch::Insert {
                        parent_id: function.id,
                        collection: ChildCollection::FunctionNodes,
                        index: node_count,
                        entity: Box::new(GraphEntity::FunctionNode(node)),
                    }
                };
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    vec![patch],
                    generation,
                    status,
                );
                on_saved.call(());
            },
                div { class: "aio-definition-dialog__grid aio-function-node-dialog__identity",
                    label {
                        span { "节点类型" }
                        Select {
                            class: "aio-input",
                            aria_label: "节点类型",
                            value: current_kind_key,
                            options: function_node_kind_options(
                                node_id,
                                &function,
                                &models,
                                &routes,
                                &functions,
                                &capabilities,
                            ),
                            on_value_change: move |key: String| {
                                match default_function_node_kind(
                                    &key,
                                    node_id,
                                    &option_function,
                                    &option_models,
                                    &option_routes,
                                    &option_functions,
                                    &option_capabilities,
                                ) {
                                    Ok(kind) => draft.with_mut(|node| node.kind = kind),
                                    Err(error) => status.set(Some(error)),
                                }
                            },
                        }
                    }
                    label {
                        span { "画布 X" }
                        Input {
                            class: "aio-input",
                            r#type: "number",
                            min: "0",
                            max: "10000",
                            aria_label: "节点画布 X",
                            value: "{current.editor.x}",
                            oninput: move |event: FormEvent| {
                                if let Ok(value) = event.value().parse::<i32>() {
                                    draft.with_mut(|node| node.editor.x = value.clamp(0, 10_000));
                                }
                            }
                        }
                    }
                    label {
                        span { "画布 Y" }
                        Input {
                            class: "aio-input",
                            r#type: "number",
                            min: "0",
                            max: "10000",
                            aria_label: "节点画布 Y",
                            value: "{current.editor.y}",
                            oninput: move |event: FormEvent| {
                                if let Ok(value) = event.value().parse::<i32>() {
                                    draft.with_mut(|node| node.editor.y = value.clamp(0, 10_000));
                                }
                            }
                        }
                    }
                }
                section { class: "aio-definition-dialog__section aio-function-node-dialog__configuration",
                    h3 { "节点配置" }
                    {function_node_editor_fields(
                        current,
                        draft,
                        function.clone(),
                        models.clone(),
                        routes.clone(),
                        functions.clone(),
                        capabilities.clone(),
                    )}
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| on_close.call(()),
                        "取消"
                    }
                    Button {
                        r#type: "submit",
                        icons::Save { class: "size-4" }
                        if editing { "保存节点" } else { "创建节点" }
                    }
                }
            }
        }
    }
}

pub(super) fn next_function_node_editor(nodes: &[FunctionNode]) -> FunctionNodeEditor {
    const COLUMN_COUNT: usize = 3;
    const X_STEP: i32 = 240;
    const Y_STEP: i32 = 160;
    const MIN_X_GAP: i32 = 220;
    const MIN_Y_GAP: i32 = 140;

    (0..)
        .map(|index| FunctionNodeEditor {
            x: ((index % COLUMN_COUNT) as i32) * X_STEP,
            y: ((index / COLUMN_COUNT) as i32) * Y_STEP,
        })
        .find(|candidate| {
            nodes.iter().all(|node| {
                (node.editor.x - candidate.x).abs() >= MIN_X_GAP
                    || (node.editor.y - candidate.y).abs() >= MIN_Y_GAP
            })
        })
        .expect("函数节点网格应始终存在空闲位置")
}

pub(super) fn function_node_kind_key(kind: &FunctionNodeKind) -> &'static str {
    match kind {
        FunctionNodeKind::Constant { .. } => "constant",
        FunctionNodeKind::Input { .. } => "input",
        FunctionNodeKind::Output { .. } => "output",
        FunctionNodeKind::Object { .. } => "object",
        FunctionNodeKind::List { .. } => "list",
        FunctionNodeKind::FieldAccess { .. } => "field_access",
        FunctionNodeKind::Format { .. } => "format",
        FunctionNodeKind::Compare { .. } => "compare",
        FunctionNodeKind::Boolean { .. } => "boolean",
        FunctionNodeKind::Math { .. } => "math",
        FunctionNodeKind::Condition => "condition",
        FunctionNodeKind::ForEach { .. } => "for_each",
        FunctionNodeKind::ValidateForm { .. } => "validate_form",
        FunctionNodeKind::CreateRecord { .. } => "create_record",
        FunctionNodeKind::ReadRecord { .. } => "read_record",
        FunctionNodeKind::UpdateRecord { .. } => "update_record",
        FunctionNodeKind::DeleteRecord { .. } => "delete_record",
        FunctionNodeKind::QueryRecords { .. } => "query_records",
        FunctionNodeKind::Navigate { .. } => "navigate",
        FunctionNodeKind::Confirm { .. } => "confirm",
        FunctionNodeKind::Notify { .. } => "notify",
        FunctionNodeKind::Return => "return",
        FunctionNodeKind::Fail { .. } => "fail",
        FunctionNodeKind::Capability { .. } => "capability",
    }
}

pub(super) fn function_node_kind_options(
    current_node_id: SymbolId,
    function: &FunctionDefinition,
    models: &[ModelDefinition],
    routes: &[RouteDefinition],
    functions: &[FunctionDefinition],
    capabilities: &crate::CapabilityCatalog,
) -> Vec<SelectItem> {
    let has_inputs = !function.inputs.is_empty();
    let has_outputs = !function.outputs.is_empty();
    let has_models = !models.is_empty();
    let has_routes = !routes.is_empty();
    let has_body_functions = functions.iter().any(|item| item.id != function.id);
    let has_reference_nodes = function
        .graph
        .nodes
        .iter()
        .any(|node| node.id != current_node_id);
    let has_model_fields = models.iter().any(|model| !model.fields.is_empty());
    vec![
        SelectItem::new("constant", "常量"),
        SelectItem::new("input", "函数输入").disabled(!has_inputs),
        SelectItem::new("output", "函数输出").disabled(!has_outputs),
        SelectItem::new("object", "对象").disabled(!has_reference_nodes || !has_model_fields),
        SelectItem::new("list", "列表").disabled(!has_reference_nodes),
        SelectItem::new("field_access", "字段读取")
            .disabled(!has_reference_nodes || !has_model_fields),
        SelectItem::new("format", "格式化"),
        SelectItem::new("compare", "比较"),
        SelectItem::new("boolean", "布尔运算"),
        SelectItem::new("math", "数学运算"),
        SelectItem::new("condition", "条件分支"),
        SelectItem::new("for_each", "遍历调用").disabled(!has_body_functions),
        SelectItem::new("validate_form", "表单校验").disabled(!has_model_fields),
        SelectItem::new("create_record", "新增记录").disabled(!has_models),
        SelectItem::new("read_record", "读取记录").disabled(!has_models),
        SelectItem::new("update_record", "更新记录").disabled(!has_models),
        SelectItem::new("delete_record", "删除记录").disabled(!has_models),
        SelectItem::new("query_records", "查询记录").disabled(!has_models),
        SelectItem::new("navigate", "页面跳转").disabled(!has_routes),
        SelectItem::new("confirm", "确认"),
        SelectItem::new("notify", "通知"),
        SelectItem::new("return", "返回"),
        SelectItem::new("fail", "失败"),
        SelectItem::new("capability", "能力调用").disabled(capabilities.capabilities.is_empty()),
    ]
}

pub(super) fn default_function_node_kind(
    key: &str,
    current_node_id: SymbolId,
    function: &FunctionDefinition,
    models: &[ModelDefinition],
    routes: &[RouteDefinition],
    functions: &[FunctionDefinition],
    capabilities: &crate::CapabilityCatalog,
) -> Result<FunctionNodeKind, String> {
    let first_model_id = || {
        models
            .first()
            .map(|model| model.id)
            .ok_or_else(|| "请先创建模型".to_owned())
    };
    let first_reference_node_id = || {
        function
            .graph
            .nodes
            .iter()
            .find(|node| node.id != current_node_id)
            .map(|node| node.id)
            .ok_or_else(|| "请先创建可引用的函数节点".to_owned())
    };
    let first_field_id = || {
        models
            .iter()
            .flat_map(|model| &model.fields)
            .next()
            .map(|field| field.id)
            .ok_or_else(|| "请先为模型创建字段".to_owned())
    };
    match key {
        "constant" => Ok(FunctionNodeKind::Constant {
            value: serde_json::Value::String(String::new()),
            value_type: ValueType::Text,
        }),
        "input" => function
            .inputs
            .first()
            .map(|port| FunctionNodeKind::Input { port_id: port.id })
            .ok_or_else(|| "请先创建输入端口".to_owned()),
        "output" => function
            .outputs
            .first()
            .map(|port| FunctionNodeKind::Output { port_id: port.id })
            .ok_or_else(|| "请先创建输出端口".to_owned()),
        "object" => Ok(FunctionNodeKind::Object {
            fields: BTreeMap::from([(first_field_id()?, first_reference_node_id()?)]),
        }),
        "list" => Ok(FunctionNodeKind::List {
            items: vec![first_reference_node_id()?],
        }),
        "field_access" => Ok(FunctionNodeKind::FieldAccess {
            object: first_reference_node_id()?,
            field_id: first_field_id()?,
        }),
        "format" => Ok(FunctionNodeKind::Format {
            template: String::new(),
            values: Vec::new(),
        }),
        "compare" => Ok(FunctionNodeKind::Compare {
            operator: CompareOperator::Equal,
        }),
        "boolean" => Ok(FunctionNodeKind::Boolean {
            operator: BooleanOperator::And,
        }),
        "math" => Ok(FunctionNodeKind::Math {
            operator: MathOperator::Add,
        }),
        "condition" => Ok(FunctionNodeKind::Condition),
        "for_each" => functions
            .iter()
            .find(|item| item.id != function.id)
            .map(|body| FunctionNodeKind::ForEach {
                max_items: 100,
                body_function_id: body.id,
            })
            .ok_or_else(|| "遍历节点需要另一个函数作为函数体".to_owned()),
        "validate_form" => Ok(FunctionNodeKind::ValidateForm {
            rules: vec![ValidationRule {
                field_id: first_field_id()?,
                rule: ValidationRuleKind::Required,
                message: "不能为空".to_owned(),
            }],
        }),
        "create_record" => Ok(FunctionNodeKind::CreateRecord {
            model_id: first_model_id()?,
        }),
        "read_record" => Ok(FunctionNodeKind::ReadRecord {
            model_id: first_model_id()?,
        }),
        "update_record" => Ok(FunctionNodeKind::UpdateRecord {
            model_id: first_model_id()?,
        }),
        "delete_record" => Ok(FunctionNodeKind::DeleteRecord {
            model_id: first_model_id()?,
        }),
        "query_records" => Ok(FunctionNodeKind::QueryRecords {
            model_id: first_model_id()?,
            limit: 100,
        }),
        "navigate" => routes
            .first()
            .map(|route| FunctionNodeKind::Navigate { route_id: route.id })
            .ok_or_else(|| "请先创建路由".to_owned()),
        "confirm" => Ok(FunctionNodeKind::Confirm {
            message: PropertyValue::text(String::new()),
        }),
        "notify" => Ok(FunctionNodeKind::Notify {
            level: NotificationLevel::Info,
        }),
        "return" => Ok(FunctionNodeKind::Return),
        "fail" => Ok(FunctionNodeKind::Fail {
            code: "FUNCTION_FAILED".to_owned(),
        }),
        "capability" => {
            capabilities
                .capabilities
                .iter()
                .next()
                .and_then(|(capability_id, capability)| {
                    capability.operations.keys().next().map(|operation| {
                        FunctionNodeKind::Capability {
                            capability_id: capability_id.clone(),
                            operation: operation.clone(),
                        }
                    })
                })
                .ok_or_else(|| "暂无已注册能力".to_owned())
        }
        _ => Err("未知函数节点类型".to_owned()),
    }
}

pub(super) fn function_node_reference_options(
    function: &FunctionDefinition,
    current_node_id: SymbolId,
) -> Vec<(SymbolId, String)> {
    function
        .graph
        .nodes
        .iter()
        .filter(|node| node.id != current_node_id)
        .map(|node| {
            let short_id = node.id.to_string().chars().take(8).collect::<String>();
            (
                node.id,
                format!(
                    "{} · {} · {}",
                    node.name,
                    function_node_kind_title(&node.kind),
                    short_id
                ),
            )
        })
        .collect()
}

pub(super) fn function_field_options(models: &[ModelDefinition]) -> Vec<(SymbolId, String)> {
    models
        .iter()
        .flat_map(|model| {
            model.fields.iter().map(|field| {
                (
                    field.id,
                    format!("{} / {} · {}", model.title, field.title, field.name),
                )
            })
        })
        .collect()
}
