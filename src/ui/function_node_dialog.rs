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
        name: format!("node_{}", node_count + 1),
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
    let option_function = function.clone();
    let option_models = models.clone();
    let option_routes = routes.clone();
    let option_functions = functions.clone();

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
                        entity: GraphEntity::FunctionNode(node),
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
                        span { "节点名称" }
                        Input {
                            class: "aio-input",
                            aria_label: "节点名称",
                            value: "{current.name}",
                            oninput: move |event: FormEvent| {
                                draft.with_mut(|node| node.name = event.value());
                            }
                        }
                    }
                    label {
                        span { "节点类型" }
                        select {
                            class: "aio-input",
                            aria_label: "节点类型",
                            value: current_kind_key,
                            onchange: move |event: FormEvent| {
                                let key = event.value();
                                match default_function_node_kind(
                                    &key,
                                    node_id,
                                    &option_function,
                                    &option_models,
                                    &option_routes,
                                    &option_functions,
                                ) {
                                    Ok(kind) => draft.with_mut(|node| node.kind = kind),
                                    Err(error) => status.set(Some(error)),
                                }
                            },
                            {function_node_kind_options(
                                current_kind_key,
                                node_id,
                                &function,
                                &models,
                                &routes,
                                &functions,
                            )}
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
    current_key: &str,
    current_node_id: SymbolId,
    function: &FunctionDefinition,
    models: &[ModelDefinition],
    routes: &[RouteDefinition],
    functions: &[FunctionDefinition],
) -> Element {
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
    rsx! {
        option { value: "constant", selected: current_key == "constant", "常量" }
        option { value: "input", selected: current_key == "input", disabled: !has_inputs, "函数输入" }
        option { value: "output", selected: current_key == "output", disabled: !has_outputs, "函数输出" }
        option { value: "object", selected: current_key == "object", disabled: !has_reference_nodes || !has_model_fields, "对象" }
        option { value: "list", selected: current_key == "list", disabled: !has_reference_nodes, "列表" }
        option { value: "field_access", selected: current_key == "field_access", disabled: !has_reference_nodes || !has_model_fields, "字段读取" }
        option { value: "format", selected: current_key == "format", "格式化" }
        option { value: "compare", selected: current_key == "compare", "比较" }
        option { value: "boolean", selected: current_key == "boolean", "布尔运算" }
        option { value: "math", selected: current_key == "math", "数学运算" }
        option { value: "condition", selected: current_key == "condition", "条件分支" }
        option { value: "for_each", selected: current_key == "for_each", disabled: !has_body_functions, "遍历调用" }
        option { value: "validate_form", selected: current_key == "validate_form", disabled: !has_model_fields, "表单校验" }
        option { value: "create_record", selected: current_key == "create_record", disabled: !has_models, "新增记录" }
        option { value: "read_record", selected: current_key == "read_record", disabled: !has_models, "读取记录" }
        option { value: "update_record", selected: current_key == "update_record", disabled: !has_models, "更新记录" }
        option { value: "delete_record", selected: current_key == "delete_record", disabled: !has_models, "删除记录" }
        option { value: "query_records", selected: current_key == "query_records", disabled: !has_models, "查询记录" }
        option { value: "navigate", selected: current_key == "navigate", disabled: !has_routes, "页面跳转" }
        option { value: "confirm", selected: current_key == "confirm", "确认" }
        option { value: "notify", selected: current_key == "notify", "通知" }
        option { value: "return", selected: current_key == "return", "返回" }
        option { value: "fail", selected: current_key == "fail", "失败" }
        option { value: "capability", selected: current_key == "capability", "能力调用" }
    }
}

pub(super) fn default_function_node_kind(
    key: &str,
    current_node_id: SymbolId,
    function: &FunctionDefinition,
    models: &[ModelDefinition],
    routes: &[RouteDefinition],
    functions: &[FunctionDefinition],
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
        "capability" => Ok(FunctionNodeKind::Capability {
            capability_id: String::new(),
            operation: String::new(),
        }),
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
