use super::*;

#[component]
pub(super) fn FunctionDefinitionDialog(
    function: Option<FunctionDefinition>,
    functions: Vec<FunctionDefinition>,
    permissions: Vec<PermissionDefinition>,
    root_id: SymbolId,
    function_count: usize,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<SymbolId>,
) -> Element {
    let editing = function.is_some();
    let function_id = function
        .as_ref()
        .map_or_else(SymbolId::new, |function| function.id);
    let initial_name = function
        .as_ref()
        .map(|function| function.name.clone())
        .unwrap_or_default();
    let initial_title = function
        .as_ref()
        .map(|function| function.title.clone())
        .unwrap_or_default();
    let selected_permissions = function
        .as_ref()
        .map(|function| function.required_permissions.clone())
        .unwrap_or_default();
    let mut name = use_signal(move || initial_name);
    let mut title = use_signal(move || initial_title);
    let existing_functions = functions;
    let save_permissions = permissions.clone();

    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-function-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑函数" } else { "新建函数" } }
                    DialogDescription { "函数身份、权限与结构化节点图共享同一正式定义" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭函数编辑",
                    aria_label: "关闭函数编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_name = name().trim().to_owned();
                let next_title = title().trim().to_owned();
                if !data_identifier_is_valid(&next_name) {
                    status.set(Some("函数标识必须采用 snake_case".to_owned()));
                    return;
                }
                if next_title.is_empty() {
                    status.set(Some("函数标题不能为空".to_owned()));
                    return;
                }
                if existing_functions
                    .iter()
                    .any(|item| item.id != function_id && item.name == next_name)
                {
                    status.set(Some(format!("函数标识已存在: {next_name}")));
                    return;
                }
                let required_permissions = function_permissions_from_form(&event, &save_permissions);
                let patches = if editing {
                    vec![
                        GraphPatch::Rename {
                            target_id: function_id,
                            name: next_name,
                            title: Some(next_title),
                        },
                        GraphPatch::SetProperty {
                            target_id: function_id,
                            property: crate::EditableProperty::FunctionPermissions,
                            value: serde_json::json!(required_permissions),
                        },
                    ]
                } else {
                    let function = FunctionDefinition {
                        id: function_id,
                        name: next_name,
                        title: next_title,
                        state: DefinitionState::Known,
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                        graph: FunctionGraph::default(),
                        required_permissions,
                    };
                    vec![GraphPatch::Insert {
                        parent_id: root_id,
                        collection: ChildCollection::Functions,
                        index: function_count,
                        entity: GraphEntity::Function(function),
                    }]
                };
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    patches,
                    generation,
                    status,
                );
                on_saved.call(function_id);
            },
                div { class: "aio-definition-dialog__grid",
                    label {
                        span { "函数标识" }
                        Input {
                            class: "aio-input",
                            aria_label: "函数标识",
                            placeholder: "例如 approve_order",
                            value: name(),
                            oninput: move |event: FormEvent| name.set(event.value()),
                        }
                    }
                    label {
                        span { "函数标题" }
                        Input {
                            class: "aio-input",
                            aria_label: "函数标题",
                            placeholder: "例如 审批工单",
                            value: title(),
                            oninput: move |event: FormEvent| title.set(event.value()),
                        }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "执行权限" }
                    if permissions.is_empty() {
                        p { class: "aio-definition-dialog__empty-state", "暂无权限定义" }
                    } else {
                        div { class: "aio-definition-dialog__choice-list",
                            for permission in &permissions {
                                label {
                                    Checkbox {
                                        name: "{function_permission_input_name(permission.id)}",
                                        default_checked: checkbox_state(selected_permissions.contains(&permission.id)),
                                        aria_label: "函数需要权限 {permission.title}",
                                    }
                                    span {
                                        strong { "{permission.title}" }
                                        code { "{permission.name}" }
                                    }
                                }
                            }
                        }
                    }
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
                        if editing { "保存函数" } else { "创建函数" }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn FunctionPortDialog(
    port: Option<PortDefinition>,
    input: bool,
    function: FunctionDefinition,
    models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let editing = port.is_some();
    let port_id = port.as_ref().map_or_else(SymbolId::new, |port| port.id);
    let current_value_type = port
        .as_ref()
        .map(|port| port.value_type.clone())
        .unwrap_or(ValueType::Text);
    let initial_name = port
        .as_ref()
        .map(|port| port.name.clone())
        .unwrap_or_default();
    let initial_type_key = function_port_type_key(&current_value_type).to_owned();
    let initial_model_id = value_type_model_id(&current_value_type)
        .map(|model_id| model_id.to_string())
        .unwrap_or_default();
    let mut name = use_signal(move || initial_name);
    let mut type_key = use_signal(move || initial_type_key);
    let mut model_id = use_signal(move || initial_model_id);
    let direction = if input { "输入" } else { "输出" };
    let collection = if input {
        ChildCollection::FunctionInputs
    } else {
        ChildCollection::FunctionOutputs
    };
    let insert_index = if input {
        function.inputs.len()
    } else {
        function.outputs.len()
    };

    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-function-port-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle {
                        if editing { "编辑{direction}端口" } else { "新建{direction}端口" }
                    }
                    DialogDescription { "{function.title} · {function.name}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭端口编辑",
                    aria_label: "关闭端口编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_name = name().trim().to_owned();
                if !data_identifier_is_valid(&next_name) {
                    status.set(Some("端口标识必须采用 snake_case".to_owned()));
                    return;
                }
                if function
                    .inputs
                    .iter()
                    .chain(&function.outputs)
                    .any(|item| item.id != port_id && item.name == next_name)
                {
                    status.set(Some(format!("端口标识已存在: {next_name}")));
                    return;
                }
                let value_type = match function_port_value_type(
                    &type_key(),
                    &model_id(),
                    &current_value_type,
                ) {
                    Ok(value_type) => value_type,
                    Err(error) => {
                        status.set(Some(error));
                        return;
                    }
                };
                let port = PortDefinition {
                    id: port_id,
                    name: next_name,
                    value_type,
                };
                let patch = if editing {
                    GraphPatch::SetProperty {
                        target_id: port_id,
                        property: crate::EditableProperty::FunctionPort,
                        value: serde_json::json!(port),
                    }
                } else {
                    GraphPatch::Insert {
                        parent_id: function.id,
                        collection,
                        index: insert_index,
                        entity: GraphEntity::Port(port),
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
                div { class: "aio-definition-dialog__grid",
                    label {
                        span { "端口标识" }
                        Input {
                            class: "aio-input",
                            aria_label: "端口标识",
                            placeholder: if input { "例如 order_id" } else { "例如 result" },
                            value: name(),
                            oninput: move |event: FormEvent| name.set(event.value()),
                        }
                    }
                    label {
                        span { "值类型" }
                        select {
                            class: "aio-input",
                            aria_label: "端口值类型",
                            value: type_key(),
                            onchange: move |event: FormEvent| type_key.set(event.value()),
                            {function_port_type_options(&type_key())}
                        }
                    }
                    if matches!(type_key().as_str(), "object" | "optional_object" | "list_object") {
                        label {
                            span { "对象模型" }
                            select {
                                class: "aio-input",
                                aria_label: "端口对象模型",
                                value: model_id(),
                                onchange: move |event: FormEvent| model_id.set(event.value()),
                                option { value: "", selected: model_id().is_empty(), "选择模型" }
                                for model in &models {
                                    option {
                                        value: "{model.id}",
                                        selected: model_id() == model.id.to_string(),
                                        "{model.title} · {model.name}"
                                    }
                                }
                            }
                        }
                    }
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
                        if editing { "保存端口" } else { "创建端口" }
                    }
                }
            }
        }
    }
}

pub(super) fn function_port_type_options(current_key: &str) -> Element {
    rsx! {
        option { value: "text", selected: current_key == "text", "文本" }
        option { value: "integer", selected: current_key == "integer", "整数" }
        option { value: "decimal", selected: current_key == "decimal", "小数" }
        option { value: "boolean", selected: current_key == "boolean", "布尔" }
        option { value: "timestamp_ms", selected: current_key == "timestamp_ms", "时间" }
        option { value: "file", selected: current_key == "file", "文件" }
        option { value: "any", selected: current_key == "any", "任意结构" }
        option { value: "object", selected: current_key == "object", "对象" }
        option { value: "optional_object", selected: current_key == "optional_object", "可选对象" }
        option { value: "list_object", selected: current_key == "list_object", "对象列表" }
        if current_key == "preserve" {
            option { value: "preserve", selected: true, "保持现有复杂类型" }
        }
    }
}

pub(super) fn function_port_type_key(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Text => "text",
        ValueType::Integer => "integer",
        ValueType::Decimal => "decimal",
        ValueType::Boolean => "boolean",
        ValueType::TimestampMs => "timestamp_ms",
        ValueType::File => "file",
        ValueType::Any => "any",
        ValueType::Object { .. } => "object",
        ValueType::Optional { value } if matches!(value.as_ref(), ValueType::Object { .. }) => {
            "optional_object"
        }
        ValueType::List { item } if matches!(item.as_ref(), ValueType::Object { .. }) => {
            "list_object"
        }
        ValueType::Null | ValueType::List { .. } | ValueType::Optional { .. } => "preserve",
    }
}

pub(super) fn value_type_model_id(value_type: &ValueType) -> Option<SymbolId> {
    match value_type {
        ValueType::Object { model_id } => Some(*model_id),
        ValueType::Optional { value } => value_type_model_id(value),
        ValueType::List { item } => value_type_model_id(item),
        ValueType::Any
        | ValueType::Null
        | ValueType::Boolean
        | ValueType::Integer
        | ValueType::Decimal
        | ValueType::Text
        | ValueType::TimestampMs
        | ValueType::File => None,
    }
}

pub(super) fn function_port_value_type(
    key: &str,
    model_id: &str,
    current: &ValueType,
) -> Result<ValueType, String> {
    let object = || {
        SymbolId::parse(model_id)
            .map(|model_id| ValueType::Object { model_id })
            .map_err(|_| "请选择对象模型".to_owned())
    };
    match key {
        "text" => Ok(ValueType::Text),
        "integer" => Ok(ValueType::Integer),
        "decimal" => Ok(ValueType::Decimal),
        "boolean" => Ok(ValueType::Boolean),
        "timestamp_ms" => Ok(ValueType::TimestampMs),
        "file" => Ok(ValueType::File),
        "any" => Ok(ValueType::Any),
        "object" => object(),
        "optional_object" => object().map(|value| ValueType::Optional {
            value: Box::new(value),
        }),
        "list_object" => object().map(|item| ValueType::List {
            item: Box::new(item),
        }),
        "preserve" => Ok(current.clone()),
        _ => Err("未知端口值类型".to_owned()),
    }
}
