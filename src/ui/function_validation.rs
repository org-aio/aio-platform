use super::*;

pub(super) fn validate_form_fields(
    rules: Vec<ValidationRule>,
    field_options: Vec<(SymbolId, String)>,
    mut draft: Signal<FunctionNode>,
) -> Element {
    let first_field_id = field_options.first().map(|(field_id, _)| *field_id);
    rsx! {
        div { class: "aio-function-node-editor-list",
            header {
                div {
                    h4 { "校验规则" }
                    Badge { variant: BadgeVariant::Outline, "{rules.len()}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::Sm,
                    variant: ButtonVariant::Outline,
                    disabled: first_field_id.is_none(),
                    onclick: move |_| {
                        if let Some(field_id) = first_field_id {
                            draft.with_mut(|node| {
                                if let FunctionNodeKind::ValidateForm { rules } = &mut node.kind {
                                    rules.push(ValidationRule {
                                        field_id,
                                        rule: ValidationRuleKind::Required,
                                        message: "不能为空".to_owned(),
                                    });
                                }
                            });
                        }
                    },
                    icons::Plus { class: "size-4" }
                    "添加规则"
                }
            }
            for (index, validation_rule) in rules.iter().cloned().enumerate() {
                div {
                    key: "validation-rule:{index}:{validation_rule.field_id}",
                    class: "aio-function-node-editor-row aio-function-node-editor-row--validation",
                    label {
                        span { "校验字段" }
                        select {
                            class: "aio-input",
                            aria_label: "校验字段 {index}",
                            value: "{validation_rule.field_id}",
                            onchange: move |event: FormEvent| {
                                let Ok(field_id) = SymbolId::parse(&event.value()) else {
                                    return;
                                };
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::ValidateForm { rules } = &mut node.kind
                                        && let Some(rule) = rules.get_mut(index)
                                    {
                                        rule.field_id = field_id;
                                    }
                                });
                            },
                            for (option_id, label) in &field_options {
                                option {
                                    value: "{option_id}",
                                    selected: *option_id == validation_rule.field_id,
                                    "{label}"
                                }
                            }
                        }
                    }
                    label {
                        span { "规则类型" }
                        select {
                            class: "aio-input",
                            aria_label: "校验规则类型 {index}",
                            value: validation_rule_kind_key(&validation_rule.rule),
                            onchange: move |event: FormEvent| {
                                let next_rule = default_validation_rule_kind(&event.value());
                                draft.with_mut(|node| {
                                    if let FunctionNodeKind::ValidateForm { rules } = &mut node.kind
                                        && let Some(rule) = rules.get_mut(index)
                                    {
                                        rule.rule = next_rule;
                                    }
                                });
                            },
                            {validation_rule_kind_options(validation_rule_kind_key(&validation_rule.rule))}
                        }
                    }
                    {validation_rule_parameter_editor(index, validation_rule.rule.clone(), draft)}
                    label {
                        span { "错误消息" }
                        Input {
                            class: "aio-input",
                            aria_label: "校验错误消息 {index}",
                            value: validation_rule.message,
                            oninput: move |event: FormEvent| draft.with_mut(|node| {
                                if let FunctionNodeKind::ValidateForm { rules } = &mut node.kind
                                    && let Some(rule) = rules.get_mut(index)
                                {
                                    rule.message = event.value();
                                }
                            }),
                        }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "移除校验规则",
                        aria_label: "移除校验规则 {index}",
                        disabled: rules.len() == 1,
                        onclick: move |_| draft.with_mut(|node| {
                            if let FunctionNodeKind::ValidateForm { rules } = &mut node.kind
                                && rules.len() > 1
                                && index < rules.len()
                            {
                                rules.remove(index);
                            }
                        }),
                        icons::Trash2 { class: "size-4" }
                    }
                }
            }
        }
    }
}

pub(super) fn validation_rule_kind_key(kind: &ValidationRuleKind) -> &'static str {
    match kind {
        ValidationRuleKind::Required => "required",
        ValidationRuleKind::MinLength { .. } => "min_length",
        ValidationRuleKind::MaxLength { .. } => "max_length",
        ValidationRuleKind::Minimum { .. } => "minimum",
        ValidationRuleKind::Maximum { .. } => "maximum",
        ValidationRuleKind::Pattern { .. } => "pattern",
    }
}

pub(super) fn default_validation_rule_kind(key: &str) -> ValidationRuleKind {
    match key {
        "min_length" => ValidationRuleKind::MinLength { value: 1 },
        "max_length" => ValidationRuleKind::MaxLength { value: 100 },
        "minimum" => ValidationRuleKind::Minimum { value: 0.0 },
        "maximum" => ValidationRuleKind::Maximum { value: 100.0 },
        "pattern" => ValidationRuleKind::Pattern {
            name: String::new(),
        },
        _ => ValidationRuleKind::Required,
    }
}

pub(super) fn validation_rule_kind_options(current_key: &str) -> Element {
    rsx! {
        option { value: "required", selected: current_key == "required", "必填" }
        option { value: "min_length", selected: current_key == "min_length", "最小长度" }
        option { value: "max_length", selected: current_key == "max_length", "最大长度" }
        option { value: "minimum", selected: current_key == "minimum", "最小值" }
        option { value: "maximum", selected: current_key == "maximum", "最大值" }
        option { value: "pattern", selected: current_key == "pattern", "命名模式" }
    }
}

pub(super) fn validation_rule_parameter_editor(
    index: usize,
    kind: ValidationRuleKind,
    draft: Signal<FunctionNode>,
) -> Element {
    match kind {
        ValidationRuleKind::Required => rsx! {
            label {
                span { "规则参数" }
                Input {
                    class: "aio-input",
                    aria_label: "校验规则参数 {index}",
                    value: "无需参数",
                    disabled: true,
                }
            }
        },
        ValidationRuleKind::MinLength { value } => rsx! {
            label {
                span { "最小长度" }
                Input {
                    class: "aio-input",
                    r#type: "number",
                    min: "0",
                    max: "100000",
                    aria_label: "校验规则参数 {index}",
                    value: "{value}",
                    oninput: move |event: FormEvent| {
                        if let Ok(value) = event.value().parse::<u32>() {
                            update_validation_rule_kind(
                                draft,
                                index,
                                ValidationRuleKind::MinLength { value },
                            );
                        }
                    }
                }
            }
        },
        ValidationRuleKind::MaxLength { value } => rsx! {
            label {
                span { "最大长度" }
                Input {
                    class: "aio-input",
                    r#type: "number",
                    min: "0",
                    max: "100000",
                    aria_label: "校验规则参数 {index}",
                    value: "{value}",
                    oninput: move |event: FormEvent| {
                        if let Ok(value) = event.value().parse::<u32>() {
                            update_validation_rule_kind(
                                draft,
                                index,
                                ValidationRuleKind::MaxLength { value },
                            );
                        }
                    }
                }
            }
        },
        ValidationRuleKind::Minimum { value } => rsx! {
            label {
                span { "最小值" }
                Input {
                    class: "aio-input",
                    r#type: "number",
                    step: "any",
                    aria_label: "校验规则参数 {index}",
                    value: "{value}",
                    oninput: move |event: FormEvent| {
                        if let Ok(value) = event.value().parse::<f64>() {
                            update_validation_rule_kind(
                                draft,
                                index,
                                ValidationRuleKind::Minimum { value },
                            );
                        }
                    }
                }
            }
        },
        ValidationRuleKind::Maximum { value } => rsx! {
            label {
                span { "最大值" }
                Input {
                    class: "aio-input",
                    r#type: "number",
                    step: "any",
                    aria_label: "校验规则参数 {index}",
                    value: "{value}",
                    oninput: move |event: FormEvent| {
                        if let Ok(value) = event.value().parse::<f64>() {
                            update_validation_rule_kind(
                                draft,
                                index,
                                ValidationRuleKind::Maximum { value },
                            );
                        }
                    }
                }
            }
        },
        ValidationRuleKind::Pattern { name } => rsx! {
            label {
                span { "模式名称" }
                Input {
                    class: "aio-input",
                    aria_label: "校验规则参数 {index}",
                    placeholder: "例如 email",
                    value: name,
                    oninput: move |event: FormEvent| {
                        update_validation_rule_kind(
                            draft,
                            index,
                            ValidationRuleKind::Pattern { name: event.value() },
                        );
                    }
                }
            }
        },
    }
}

pub(super) fn update_validation_rule_kind(
    mut draft: Signal<FunctionNode>,
    index: usize,
    next_kind: ValidationRuleKind,
) {
    draft.with_mut(|node| {
        if let FunctionNodeKind::ValidateForm { rules } = &mut node.kind
            && let Some(rule) = rules.get_mut(index)
        {
            rule.rule = next_kind;
        }
    });
}

pub(super) fn function_constant_text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub(super) fn function_constant_value(text: &str, value_type: &ValueType) -> serde_json::Value {
    match value_type {
        ValueType::Integer => text
            .parse::<i64>()
            .map(serde_json::Value::from)
            .unwrap_or_else(|_| serde_json::Value::String(text.to_owned())),
        ValueType::Decimal => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::String(text.to_owned())),
        ValueType::Boolean => text
            .parse::<bool>()
            .map(serde_json::Value::Bool)
            .unwrap_or_else(|_| serde_json::Value::String(text.to_owned())),
        ValueType::Null => serde_json::Value::Null,
        ValueType::Any
        | ValueType::Text
        | ValueType::TimestampMs
        | ValueType::File
        | ValueType::Object { .. }
        | ValueType::List { .. }
        | ValueType::Optional { .. } => serde_json::Value::String(text.to_owned()),
    }
}

pub(super) fn function_record_node_kind(key: &str, model_id: SymbolId) -> FunctionNodeKind {
    match key {
        "create_record" => FunctionNodeKind::CreateRecord { model_id },
        "read_record" => FunctionNodeKind::ReadRecord { model_id },
        "update_record" => FunctionNodeKind::UpdateRecord { model_id },
        _ => FunctionNodeKind::DeleteRecord { model_id },
    }
}

pub(super) fn compare_operator_key(operator: CompareOperator) -> &'static str {
    match operator {
        CompareOperator::Equal => "equal",
        CompareOperator::NotEqual => "not_equal",
        CompareOperator::Greater => "greater",
        CompareOperator::GreaterOrEqual => "greater_or_equal",
        CompareOperator::Less => "less",
        CompareOperator::LessOrEqual => "less_or_equal",
        CompareOperator::Contains => "contains",
    }
}

pub(super) fn compare_operator_from_key(key: &str) -> CompareOperator {
    match key {
        "not_equal" => CompareOperator::NotEqual,
        "greater" => CompareOperator::Greater,
        "greater_or_equal" => CompareOperator::GreaterOrEqual,
        "less" => CompareOperator::Less,
        "less_or_equal" => CompareOperator::LessOrEqual,
        "contains" => CompareOperator::Contains,
        _ => CompareOperator::Equal,
    }
}

pub(super) fn compare_operator_options(current: CompareOperator) -> Element {
    rsx! {
        option { value: "equal", selected: current == CompareOperator::Equal, "等于" }
        option { value: "not_equal", selected: current == CompareOperator::NotEqual, "不等于" }
        option { value: "greater", selected: current == CompareOperator::Greater, "大于" }
        option { value: "greater_or_equal", selected: current == CompareOperator::GreaterOrEqual, "大于等于" }
        option { value: "less", selected: current == CompareOperator::Less, "小于" }
        option { value: "less_or_equal", selected: current == CompareOperator::LessOrEqual, "小于等于" }
        option { value: "contains", selected: current == CompareOperator::Contains, "包含" }
    }
}

pub(super) fn boolean_operator_key(operator: BooleanOperator) -> &'static str {
    match operator {
        BooleanOperator::And => "and",
        BooleanOperator::Or => "or",
        BooleanOperator::Not => "not",
    }
}

pub(super) fn boolean_operator_from_key(key: &str) -> BooleanOperator {
    match key {
        "or" => BooleanOperator::Or,
        "not" => BooleanOperator::Not,
        _ => BooleanOperator::And,
    }
}

pub(super) fn boolean_operator_options(current: BooleanOperator) -> Element {
    rsx! {
        option { value: "and", selected: current == BooleanOperator::And, "并且" }
        option { value: "or", selected: current == BooleanOperator::Or, "或者" }
        option { value: "not", selected: current == BooleanOperator::Not, "取反" }
    }
}

pub(super) fn math_operator_key(operator: MathOperator) -> &'static str {
    match operator {
        MathOperator::Add => "add",
        MathOperator::Subtract => "subtract",
        MathOperator::Multiply => "multiply",
        MathOperator::Divide => "divide",
        MathOperator::Remainder => "remainder",
    }
}

pub(super) fn math_operator_from_key(key: &str) -> MathOperator {
    match key {
        "subtract" => MathOperator::Subtract,
        "multiply" => MathOperator::Multiply,
        "divide" => MathOperator::Divide,
        "remainder" => MathOperator::Remainder,
        _ => MathOperator::Add,
    }
}

pub(super) fn math_operator_options(current: MathOperator) -> Element {
    rsx! {
        option { value: "add", selected: current == MathOperator::Add, "加" }
        option { value: "subtract", selected: current == MathOperator::Subtract, "减" }
        option { value: "multiply", selected: current == MathOperator::Multiply, "乘" }
        option { value: "divide", selected: current == MathOperator::Divide, "除" }
        option { value: "remainder", selected: current == MathOperator::Remainder, "取余" }
    }
}

pub(super) fn notification_level_key(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "info",
        NotificationLevel::Success => "success",
        NotificationLevel::Warning => "warning",
        NotificationLevel::Error => "error",
    }
}

pub(super) fn notification_level_from_key(key: &str) -> NotificationLevel {
    match key {
        "success" => NotificationLevel::Success,
        "warning" => NotificationLevel::Warning,
        "error" => NotificationLevel::Error,
        _ => NotificationLevel::Info,
    }
}

pub(super) fn notification_level_options(current: NotificationLevel) -> Element {
    rsx! {
        option { value: "info", selected: current == NotificationLevel::Info, "信息" }
        option { value: "success", selected: current == NotificationLevel::Success, "成功" }
        option { value: "warning", selected: current == NotificationLevel::Warning, "警告" }
        option { value: "error", selected: current == NotificationLevel::Error, "错误" }
    }
}
