use super::*;

pub(super) fn function_node_editor_fields(
    node: FunctionNode,
    mut draft: Signal<FunctionNode>,
    function: FunctionDefinition,
    models: Vec<ModelDefinition>,
    routes: Vec<RouteDefinition>,
    functions: Vec<FunctionDefinition>,
) -> Element {
    let current_node_id = node.id;
    let reference_nodes = function_node_reference_options(&function, current_node_id);
    let field_options = function_field_options(&models);
    match node.kind {
        FunctionNodeKind::Constant { value, value_type } => {
            let value_text = function_constant_text(&value);
            rsx! {
                div { class: "aio-definition-dialog__grid",
                    label {
                        span { "常量值" }
                        Input {
                            class: "aio-input",
                            aria_label: "常量值",
                            value: value_text,
                            oninput: move |event: FormEvent| {
                                let text = event.value();
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::Constant { value, value_type } = &mut node.kind {
                                        *value = function_constant_value(&text, value_type);
                                    }
                                });
                            }
                        }
                    }
                    label {
                        span { "常量类型" }
                        select {
                            class: "aio-input",
                            aria_label: "常量类型",
                            value: editable_value_type_key(&value_type),
                            onchange: move |event: FormEvent| {
                                let next_type = value_type_from_key(&event.value());
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::Constant { value, value_type } = &mut node.kind {
                                        let text = function_constant_text(value);
                                        *value = function_constant_value(&text, &next_type);
                                        *value_type = next_type;
                                    }
                                });
                            },
                            {editable_value_type_options(
                                &value_type,
                                editable_value_type_key(&value_type).to_owned(),
                            )}
                        }
                    }
                }
            }
        }
        FunctionNodeKind::Input { port_id } => rsx! {
            label { "输入端口"
                select {
                    class: "aio-input",
                    aria_label: "输入端口",
                    value: "{port_id}",
                    onchange: move |event: FormEvent| {
                        if let Ok(port_id) = SymbolId::parse(&event.value()) {
                            draft.with_mut(|node| node.kind = FunctionNodeKind::Input { port_id });
                        }
                    },
                    for port in &function.inputs {
                        option {
                            value: "{port.id}",
                            selected: port.id == port_id,
                            "{port.name} · {value_type_label(&port.value_type)}"
                        }
                    }
                }
            }
        },
        FunctionNodeKind::Output { port_id } => rsx! {
            label { "输出端口"
                select {
                    class: "aio-input",
                    aria_label: "输出端口",
                    value: "{port_id}",
                    onchange: move |event: FormEvent| {
                        if let Ok(port_id) = SymbolId::parse(&event.value()) {
                            draft.with_mut(|node| node.kind = FunctionNodeKind::Output { port_id });
                        }
                    },
                    for port in &function.outputs {
                        option {
                            value: "{port.id}",
                            selected: port.id == port_id,
                            "{port.name} · {value_type_label(&port.value_type)}"
                        }
                    }
                }
            }
        },
        FunctionNodeKind::Compare { operator } => rsx! {
            label { "比较操作"
                select {
                    class: "aio-input",
                    aria_label: "比较操作",
                    value: compare_operator_key(operator),
                    onchange: move |event: FormEvent| {
                        draft.with_mut(|node| {
                            node.kind = FunctionNodeKind::Compare {
                                operator: compare_operator_from_key(&event.value()),
                            };
                        });
                    },
                    {compare_operator_options(operator)}
                }
            }
        },
        FunctionNodeKind::Boolean { operator } => rsx! {
            label { "布尔操作"
                select {
                    class: "aio-input",
                    aria_label: "布尔操作",
                    value: boolean_operator_key(operator),
                    onchange: move |event: FormEvent| {
                        draft.with_mut(|node| {
                            node.kind = FunctionNodeKind::Boolean {
                                operator: boolean_operator_from_key(&event.value()),
                            };
                        });
                    },
                    {boolean_operator_options(operator)}
                }
            }
        },
        FunctionNodeKind::Math { operator } => rsx! {
            label { "数学操作"
                select {
                    class: "aio-input",
                    aria_label: "数学操作",
                    value: math_operator_key(operator),
                    onchange: move |event: FormEvent| {
                        draft.with_mut(|node| {
                            node.kind = FunctionNodeKind::Math {
                                operator: math_operator_from_key(&event.value()),
                            };
                        });
                    },
                    {math_operator_options(operator)}
                }
            }
        },
        FunctionNodeKind::ForEach {
            max_items,
            body_function_id,
        } => rsx! {
            div { class: "aio-definition-dialog__grid",
                label { "函数体"
                    select {
                        class: "aio-input",
                        aria_label: "遍历函数体",
                        value: "{body_function_id}",
                        onchange: move |event: FormEvent| {
                            if let Ok(body_function_id) = SymbolId::parse(&event.value()) {
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::ForEach { body_function_id: current, .. } = &mut node.kind {
                                        *current = body_function_id;
                                    }
                                });
                            }
                        },
                        for item in functions.iter().filter(|item| item.id != function.id) {
                            option {
                                value: "{item.id}",
                                selected: item.id == body_function_id,
                                "{item.title} · {item.name}"
                            }
                        }
                    }
                }
                label { "最大项数"
                    Input {
                        class: "aio-input",
                        r#type: "number",
                        min: "1",
                        max: "10000",
                        aria_label: "遍历最大项数",
                        value: "{max_items}",
                        oninput: move |event: FormEvent| {
                            if let Ok(value) = event.value().parse::<u32>() {
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::ForEach { max_items, .. } = &mut node.kind {
                                        *max_items = value.clamp(1, 10_000);
                                    }
                                });
                            }
                        }
                    }
                }
            }
        },
        FunctionNodeKind::CreateRecord { model_id }
        | FunctionNodeKind::ReadRecord { model_id }
        | FunctionNodeKind::UpdateRecord { model_id }
        | FunctionNodeKind::DeleteRecord { model_id } => {
            let kind_key = function_node_kind_key(&node.kind).to_owned();
            rsx! {
                label { "数据模型"
                    select {
                        class: "aio-input",
                        aria_label: "节点数据模型",
                        value: "{model_id}",
                        onchange: move |event: FormEvent| {
                            if let Ok(model_id) = SymbolId::parse(&event.value()) {
                                draft.with_mut(|node| {
                                    node.kind = function_record_node_kind(&kind_key, model_id);
                                });
                            }
                        },
                        for model in &models {
                            option {
                                value: "{model.id}",
                                selected: model.id == model_id,
                                "{model.title} · {model.name}"
                            }
                        }
                    }
                }
            }
        }
        FunctionNodeKind::QueryRecords { model_id, limit } => rsx! {
            div { class: "aio-definition-dialog__grid",
                label { "数据模型"
                    select {
                        class: "aio-input",
                        aria_label: "查询数据模型",
                        value: "{model_id}",
                        onchange: move |event: FormEvent| {
                            if let Ok(model_id) = SymbolId::parse(&event.value()) {
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::QueryRecords { model_id: current, .. } = &mut node.kind {
                                        *current = model_id;
                                    }
                                });
                            }
                        },
                        for model in &models {
                            option {
                                value: "{model.id}",
                                selected: model.id == model_id,
                                "{model.title} · {model.name}"
                            }
                        }
                    }
                }
                label { "最大记录数"
                    Input {
                        class: "aio-input",
                        r#type: "number",
                        min: "1",
                        max: "10000",
                        aria_label: "查询最大记录数",
                        value: "{limit}",
                        oninput: move |event: FormEvent| {
                            if let Ok(value) = event.value().parse::<u32>() {
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::QueryRecords { limit, .. } = &mut node.kind {
                                        *limit = value.clamp(1, 10_000);
                                    }
                                });
                            }
                        }
                    }
                }
            }
        },
        FunctionNodeKind::Navigate { route_id } => rsx! {
            label { "目标路由"
                select {
                    class: "aio-input",
                    aria_label: "节点目标路由",
                    value: "{route_id}",
                    onchange: move |event: FormEvent| {
                        if let Ok(route_id) = SymbolId::parse(&event.value()) {
                            draft.with_mut(|node| node.kind = FunctionNodeKind::Navigate { route_id });
                        }
                    },
                    for route in &routes {
                        option {
                            value: "{route.id}",
                            selected: route.id == route_id,
                            "{route.path} · {route.name}"
                        }
                    }
                }
            }
        },
        FunctionNodeKind::Confirm { message } => {
            let message_text = match message {
                PropertyValue::Literal { value } => function_constant_text(&value),
                PropertyValue::EventValue { name } => name,
            };
            rsx! {
                label { "确认消息"
                    Input {
                        class: "aio-input",
                        aria_label: "确认消息",
                        value: message_text,
                        oninput: move |event: FormEvent| {
                            draft.with_mut(|node| {
                                node.kind = FunctionNodeKind::Confirm {
                                    message: PropertyValue::text(event.value()),
                                };
                            });
                        }
                    }
                }
            }
        }
        FunctionNodeKind::Notify { level } => rsx! {
            label { "通知级别"
                select {
                    class: "aio-input",
                    aria_label: "通知级别",
                    value: notification_level_key(level),
                    onchange: move |event: FormEvent| {
                        draft.with_mut(|node| {
                            node.kind = FunctionNodeKind::Notify {
                                level: notification_level_from_key(&event.value()),
                            };
                        });
                    },
                    {notification_level_options(level)}
                }
            }
        },
        FunctionNodeKind::Fail { code } => rsx! {
            label { "失败代码"
                Input {
                    class: "aio-input",
                    aria_label: "失败代码",
                    value: code,
                    oninput: move |event: FormEvent| {
                        draft.with_mut(|node| {
                            node.kind = FunctionNodeKind::Fail { code: event.value() };
                        });
                    }
                }
            }
        },
        FunctionNodeKind::Capability {
            capability_id,
            operation,
        } => rsx! {
            div { class: "aio-definition-dialog__grid",
                label { "能力标识"
                    Input {
                        class: "aio-input",
                        aria_label: "能力标识",
                        value: capability_id,
                        oninput: move |event: FormEvent| {
                            let capability_id = event.value();
                            draft.with_mut(|node| {
                                if let FunctionNodeKind::Capability { capability_id: current, .. } = &mut node.kind {
                                    *current = capability_id;
                                }
                            });
                        }
                    }
                }
                label { "能力操作"
                    Input {
                        class: "aio-input",
                        aria_label: "能力操作",
                        value: operation,
                        oninput: move |event: FormEvent| {
                            let operation = event.value();
                            draft.with_mut(|node| {
                                if let FunctionNodeKind::Capability { operation: current, .. } = &mut node.kind {
                                    *current = operation;
                                }
                            });
                        }
                    }
                }
            }
        },
        FunctionNodeKind::Object { fields } => {
            let next_field_id = field_options
                .iter()
                .find(|(field_id, _)| !fields.contains_key(field_id))
                .map(|(field_id, _)| *field_id);
            let next_value_node_id = reference_nodes.first().map(|(node_id, _)| *node_id);
            let can_add = next_field_id.is_some() && next_value_node_id.is_some();
            rsx! {
                div { class: "aio-function-node-editor-list",
                    header {
                        div {
                            h4 { "字段映射" }
                            Badge { variant: BadgeVariant::Outline, "{fields.len()}" }
                        }
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            variant: ButtonVariant::Outline,
                            disabled: !can_add,
                            onclick: move |_| {
                                if let (Some(field_id), Some(value_node_id)) =
                                    (next_field_id, next_value_node_id)
                                {
                                    draft.with_mut(|node| {
                                        if let FunctionNodeKind::Object { fields } = &mut node.kind {
                                            fields.insert(field_id, value_node_id);
                                        }
                                    });
                                }
                            },
                            icons::Plus { class: "size-4" }
                            "添加映射"
                        }
                    }
                    if fields.is_empty() {
                        p { class: "aio-definition-dialog__empty-state", "暂无字段映射" }
                    }
                    for (field_id, value_node_id) in fields.iter().map(|(field_id, value_node_id)| (*field_id, *value_node_id)) {
                        div {
                            key: "object-field:{field_id}",
                            class: "aio-function-node-editor-row",
                            label {
                                span { "目标字段" }
                                select {
                                    class: "aio-input",
                                    aria_label: "对象目标字段 {field_id}",
                                    value: "{field_id}",
                                    onchange: move |event: FormEvent| {
                                        let Ok(next_field_id) = SymbolId::parse(&event.value()) else {
                                            return;
                                        };
                                        draft.with_mut(|node| {
                                            let FunctionNodeKind::Object { fields } = &mut node.kind else {
                                                return;
                                            };
                                            if next_field_id == field_id || fields.contains_key(&next_field_id) {
                                                return;
                                            }
                                            if let Some(value_node_id) = fields.remove(&field_id) {
                                                fields.insert(next_field_id, value_node_id);
                                            }
                                        });
                                    },
                                    for (option_id, label) in &field_options {
                                        option {
                                            value: "{option_id}",
                                            selected: *option_id == field_id,
                                            disabled: *option_id != field_id && fields.contains_key(option_id),
                                            "{label}"
                                        }
                                    }
                                }
                            }
                            label {
                                span { "来源节点" }
                                select {
                                    class: "aio-input",
                                    aria_label: "对象来源节点 {field_id}",
                                    value: "{value_node_id}",
                                    onchange: move |event: FormEvent| {
                                        let Ok(next_node_id) = SymbolId::parse(&event.value()) else {
                                            return;
                                        };
                                        draft.with_mut(|node| {
                                            if let FunctionNodeKind::Object { fields } = &mut node.kind
                                                && let Some(value_node_id) = fields.get_mut(&field_id)
                                            {
                                                *value_node_id = next_node_id;
                                            }
                                        });
                                    },
                                    for (option_id, label) in &reference_nodes {
                                        option { value: "{option_id}", selected: *option_id == value_node_id, "{label}" }
                                    }
                                }
                            }
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "移除字段映射",
                                aria_label: "移除字段映射 {field_id}",
                                onclick: move |_| draft.with_mut(|node| {
                                    if let FunctionNodeKind::Object { fields } = &mut node.kind {
                                        fields.remove(&field_id);
                                    }
                                }),
                                icons::Trash2 { class: "size-4" }
                            }
                        }
                    }
                }
            }
        }
        FunctionNodeKind::List { items } => {
            let next_item_node_id = reference_nodes.first().map(|(node_id, _)| *node_id);
            rsx! {
                div { class: "aio-function-node-editor-list",
                    header {
                        div {
                            h4 { "列表元素" }
                            Badge { variant: BadgeVariant::Outline, "{items.len()}" }
                        }
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            variant: ButtonVariant::Outline,
                            disabled: next_item_node_id.is_none(),
                            onclick: move |_| {
                                if let Some(item_node_id) = next_item_node_id {
                                    draft.with_mut(|node| {
                                        if let FunctionNodeKind::List { items } = &mut node.kind {
                                            items.push(item_node_id);
                                        }
                                    });
                                }
                            },
                            icons::Plus { class: "size-4" }
                            "添加元素"
                        }
                    }
                    if items.is_empty() {
                        p { class: "aio-definition-dialog__empty-state", "暂无列表元素" }
                    }
                    for (index, item_node_id) in items.iter().copied().enumerate() {
                        div {
                            key: "list-item:{index}:{item_node_id}",
                            class: "aio-function-node-editor-row aio-function-node-editor-row--single",
                            label {
                                span { "元素 {index + 1}" }
                                select {
                                    class: "aio-input",
                                    aria_label: "列表元素节点 {index}",
                                    value: "{item_node_id}",
                                    onchange: move |event: FormEvent| {
                                        let Ok(next_node_id) = SymbolId::parse(&event.value()) else {
                                            return;
                                        };
                                        draft.with_mut(|node| {
                                            if let FunctionNodeKind::List { items } = &mut node.kind
                                                && let Some(item_node_id) = items.get_mut(index)
                                            {
                                                *item_node_id = next_node_id;
                                            }
                                        });
                                    },
                                    for (option_id, label) in &reference_nodes {
                                        option { value: "{option_id}", selected: *option_id == item_node_id, "{label}" }
                                    }
                                }
                            }
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "移除列表元素",
                                aria_label: "移除列表元素 {index}",
                                onclick: move |_| draft.with_mut(|node| {
                                    if let FunctionNodeKind::List { items } = &mut node.kind
                                        && index < items.len()
                                    {
                                        items.remove(index);
                                    }
                                }),
                                icons::Trash2 { class: "size-4" }
                            }
                        }
                    }
                }
            }
        }
        FunctionNodeKind::FieldAccess { object, field_id } => rsx! {
            div { class: "aio-definition-dialog__grid",
                label {
                    span { "对象节点" }
                    select {
                        class: "aio-input",
                        aria_label: "字段读取对象节点",
                        value: "{object}",
                        onchange: move |event: FormEvent| {
                            let Ok(object) = SymbolId::parse(&event.value()) else {
                                return;
                            };
                            draft.with_mut(|node| {
                                if let FunctionNodeKind::FieldAccess { object: current, .. } = &mut node.kind {
                                    *current = object;
                                }
                            });
                        },
                        for (option_id, label) in &reference_nodes {
                            option { value: "{option_id}", selected: *option_id == object, "{label}" }
                        }
                    }
                }
                label {
                    span { "读取字段" }
                    select {
                        class: "aio-input",
                        aria_label: "字段读取目标字段",
                        value: "{field_id}",
                        onchange: move |event: FormEvent| {
                            let Ok(field_id) = SymbolId::parse(&event.value()) else {
                                return;
                            };
                            draft.with_mut(|node| {
                                if let FunctionNodeKind::FieldAccess { field_id: current, .. } = &mut node.kind {
                                    *current = field_id;
                                }
                            });
                        },
                        for (option_id, label) in &field_options {
                            option { value: "{option_id}", selected: *option_id == field_id, "{label}" }
                        }
                    }
                }
            }
        },
        FunctionNodeKind::Format { template, values } => {
            let next_value_node_id = reference_nodes.first().map(|(node_id, _)| *node_id);
            rsx! {
                div { class: "aio-function-node-editor-stack",
                    label {
                        span { "格式模板" }
                        Textarea {
                            class: "aio-input",
                            rows: "3",
                            aria_label: "格式化模板",
                            placeholder: "例如：订单 {{0}} 已由 {{1}} 处理",
                            value: template,
                            oninput: move |event: FormEvent| {
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::Format { template, .. } = &mut node.kind {
                                        *template = event.value();
                                    }
                                });
                            }
                        }
                    }
                    div { class: "aio-function-node-editor-list",
                        header {
                            div {
                                h4 { "模板参数" }
                                Badge { variant: BadgeVariant::Outline, "{values.len()}" }
                            }
                            Button {
                                r#type: "button",
                                size: ButtonSize::Sm,
                                variant: ButtonVariant::Outline,
                                disabled: next_value_node_id.is_none(),
                                onclick: move |_| {
                                    if let Some(value_node_id) = next_value_node_id {
                                        draft.with_mut(|node| {
                                            if let FunctionNodeKind::Format { values, .. } = &mut node.kind {
                                                values.push(value_node_id);
                                            }
                                        });
                                    }
                                },
                                icons::Plus { class: "size-4" }
                                "添加参数"
                            }
                        }
                        if values.is_empty() {
                            p { class: "aio-definition-dialog__empty-state", "暂无模板参数" }
                        }
                        for (index, value_node_id) in values.iter().copied().enumerate() {
                            div {
                                key: "format-value:{index}:{value_node_id}",
                                class: "aio-function-node-editor-row aio-function-node-editor-row--single",
                                label {
                                    span { "参数 {index}" }
                                    select {
                                        class: "aio-input",
                                        aria_label: "格式化参数节点 {index}",
                                        value: "{value_node_id}",
                                        onchange: move |event: FormEvent| {
                                            let Ok(next_node_id) = SymbolId::parse(&event.value()) else {
                                                return;
                                            };
                                            draft.with_mut(|node| {
                                                if let FunctionNodeKind::Format { values, .. } = &mut node.kind
                                                    && let Some(value_node_id) = values.get_mut(index)
                                                {
                                                    *value_node_id = next_node_id;
                                                }
                                            });
                                        },
                                        for (option_id, label) in &reference_nodes {
                                            option { value: "{option_id}", selected: *option_id == value_node_id, "{label}" }
                                        }
                                    }
                                }
                                Button {
                                    r#type: "button",
                                    size: ButtonSize::IconSm,
                                    variant: ButtonVariant::Ghost,
                                    title: "移除格式化参数",
                                    aria_label: "移除格式化参数 {index}",
                                    onclick: move |_| draft.with_mut(|node| {
                                        if let FunctionNodeKind::Format { values, .. } = &mut node.kind
                                            && index < values.len()
                                        {
                                            values.remove(index);
                                        }
                                    }),
                                    icons::Trash2 { class: "size-4" }
                                }
                            }
                        }
                    }
                }
            }
        }
        FunctionNodeKind::ValidateForm { rules } => {
            validate_form_fields(rules, field_options, draft)
        }
        FunctionNodeKind::Condition | FunctionNodeKind::Return => rsx! {
            p { class: "aio-definition-dialog__empty-state", "无需附加参数" }
        },
    }
}
