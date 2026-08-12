use super::*;

#[component]
pub(super) fn ValidationEditorDialog(
    model_id: SymbolId,
    validation_count: usize,
    fields: Vec<FieldDefinition>,
    validation: Option<crate::ModelValidationDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let editing = validation.is_some();
    let validation_id = validation
        .as_ref()
        .map_or_else(SymbolId::new, |validation| validation.id);
    let initial_kind = validation
        .as_ref()
        .map(|validation| model_validation_key(&validation.rule).to_owned())
        .unwrap_or_else(|| "required_when_present".to_owned());
    let initial_message = validation
        .as_ref()
        .map(|validation| validation.message.clone())
        .unwrap_or_default();
    let (initial_field, initial_when_field, initial_selected) = validation
        .as_ref()
        .map(|validation| match &validation.rule {
            crate::ModelValidationRule::FieldsRequiredTogether { field_ids }
            | crate::ModelValidationRule::AtLeastOneRequired { field_ids } => (
                String::new(),
                String::new(),
                field_ids.iter().copied().collect::<BTreeSet<_>>(),
            ),
            crate::ModelValidationRule::RequiredWhenPresent {
                field_id,
                when_field_id,
            } => (
                field_id.to_string(),
                when_field_id.to_string(),
                BTreeSet::new(),
            ),
        })
        .unwrap_or_default();
    let mut kind = use_signal(move || initial_kind);
    let mut message = use_signal(move || initial_message);
    let mut field_id = use_signal(move || initial_field);
    let mut when_field_id = use_signal(move || initial_when_field);
    let mut selected_fields = use_signal(move || initial_selected);
    let is_conditional = kind() == "required_when_present";
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-validation-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    editor.set(None);
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑模型校验" } else { "新建模型校验" } }
                    DialogDescription { "声明跨字段依赖，不与单字段格式校验混用" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭模型校验编辑",
                    aria_label: "关闭模型校验编辑",
                    onclick: move |_| editor.set(None),
                    icons::X { class: "size-4" }
                }
            }
            form {
                class: "aio-definition-dialog__form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let next_message = message().trim().to_owned();
                    if next_message.is_empty() {
                        status.set(Some("校验失败提示不能为空".to_owned()));
                        return;
                    }
                    let rule = if kind() == "required_when_present" {
                        let Ok(selected_field_id) = SymbolId::parse(&field_id()) else {
                            status.set(Some("请选择必填字段".to_owned()));
                            return;
                        };
                        let Ok(selected_when_field_id) = SymbolId::parse(&when_field_id()) else {
                            status.set(Some("请选择条件字段".to_owned()));
                            return;
                        };
                        if selected_field_id == selected_when_field_id {
                            status.set(Some("必填字段与条件字段不能相同".to_owned()));
                            return;
                        }
                        crate::ModelValidationRule::RequiredWhenPresent {
                            field_id: selected_field_id,
                            when_field_id: selected_when_field_id,
                        }
                    } else {
                        let selected = selected_fields();
                        let ordered_fields = fields
                            .iter()
                            .filter(|field| selected.contains(&field.id))
                            .map(|field| field.id)
                            .collect::<Vec<_>>();
                        if ordered_fields.len() < 2 {
                            status.set(Some("组合校验至少需要两个字段".to_owned()));
                            return;
                        }
                        if kind() == "fields_required_together" {
                            crate::ModelValidationRule::FieldsRequiredTogether {
                                field_ids: ordered_fields,
                            }
                        } else {
                            crate::ModelValidationRule::AtLeastOneRequired {
                                field_ids: ordered_fields,
                            }
                        }
                    };
                    let definition = crate::ModelValidationDefinition {
                        id: validation_id,
                        message: next_message,
                        rule,
                    };
                    let patches = if editing {
                        vec![GraphPatch::SetProperty {
                            target_id: validation_id,
                            property: crate::EditableProperty::ModelValidation,
                            value: serde_json::json!(definition),
                        }]
                    } else {
                        vec![GraphPatch::Insert {
                            parent_id: model_id,
                            collection: ChildCollection::ModelValidations,
                            index: validation_count,
                            entity: GraphEntity::ModelValidation(definition),
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
                    editor.set(None);
                },
                label {
                    span { "校验规则" }
                    select {
                        class: "aio-input",
                        aria_label: "模型校验规则",
                        value: kind(),
                        onchange: move |event: FormEvent| kind.set(event.value()),
                        option { value: "required_when_present", "条件必填" }
                        option { value: "fields_required_together", "联合必填" }
                        option { value: "at_least_one_required", "至少一个必填" }
                    }
                }
                if is_conditional {
                    div { class: "aio-definition-dialog__grid",
                        label {
                            span { "必填字段" }
                            select {
                                class: "aio-input",
                                aria_label: "条件必填字段",
                                value: field_id(),
                                onchange: move |event: FormEvent| field_id.set(event.value()),
                                option { value: "", "选择字段" }
                                for field in &fields {
                                    option { value: "{field.id}", "{field.title} · {field.name}" }
                                }
                            }
                        }
                        label {
                            span { "条件字段" }
                            select {
                                class: "aio-input",
                                aria_label: "触发必填的条件字段",
                                value: when_field_id(),
                                onchange: move |event: FormEvent| when_field_id.set(event.value()),
                                option { value: "", "选择字段" }
                                for field in &fields {
                                    option { value: "{field.id}", "{field.title} · {field.name}" }
                                }
                            }
                        }
                    }
                } else {
                    section { class: "aio-definition-dialog__section",
                        h3 { "参与字段" }
                        div { class: "aio-definition-dialog__choice-list",
                            for field in &fields {
                                label {
                                    Checkbox {
                                        aria_label: "校验字段 {field.title}",
                                        checked: Some(checkbox_state(selected_fields().contains(&field.id))),
                                        on_checked_change: {
                                            let field_id = field.id;
                                            move |checked| selected_fields.with_mut(|selected| {
                                                if checkbox_is_checked(checked) {
                                                    selected.insert(field_id);
                                                } else {
                                                    selected.remove(&field_id);
                                                }
                                            })
                                        },
                                    }
                                    span { strong { "{field.title}" } code { "{field.name}" } }
                                }
                            }
                        }
                    }
                }
                label {
                    span { "失败提示" }
                    Textarea {
                        class: "aio-input",
                        aria_label: "模型校验失败提示",
                        rows: "3",
                        placeholder: "例如：开始时间存在时，结束时间不能为空",
                        value: message(),
                        oninput: move |event: FormEvent| message.set(event.value()),
                    }
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| editor.set(None),
                        "取消"
                    }
                    Button {
                        r#type: "submit",
                        icons::Save { class: "size-4" }
                        if editing { "保存校验" } else { "创建校验" }
                    }
                }
            }
        }
    }
}

pub(super) fn model_validation_key(rule: &crate::ModelValidationRule) -> &'static str {
    match rule {
        crate::ModelValidationRule::FieldsRequiredTogether { .. } => "fields_required_together",
        crate::ModelValidationRule::AtLeastOneRequired { .. } => "at_least_one_required",
        crate::ModelValidationRule::RequiredWhenPresent { .. } => "required_when_present",
    }
}

pub(super) fn query_conjunction_from_key(value: &str) -> crate::QueryConjunction {
    match value {
        "any" => crate::QueryConjunction::Any,
        _ => crate::QueryConjunction::All,
    }
}

pub(super) fn query_operator_from_key(value: &str) -> crate::QueryOperator {
    match value {
        "equals" => crate::QueryOperator::Equals,
        "not_equals" => crate::QueryOperator::NotEquals,
        "starts_with" => crate::QueryOperator::StartsWith,
        "ends_with" => crate::QueryOperator::EndsWith,
        "greater_than" => crate::QueryOperator::GreaterThan,
        "greater_or_equal" => crate::QueryOperator::GreaterOrEqual,
        "less_than" => crate::QueryOperator::LessThan,
        "less_or_equal" => crate::QueryOperator::LessOrEqual,
        _ => crate::QueryOperator::Contains,
    }
}

pub(super) fn query_operator_select_items() -> Vec<SelectItem> {
    vec![
        SelectItem::new("contains", "包含"),
        SelectItem::new("equals", "等于"),
        SelectItem::new("not_equals", "不等于"),
        SelectItem::new("starts_with", "开头是"),
        SelectItem::new("ends_with", "结尾是"),
        SelectItem::new("greater_than", "大于"),
        SelectItem::new("greater_or_equal", "大于等于"),
        SelectItem::new("less_than", "小于"),
        SelectItem::new("less_or_equal", "小于等于"),
    ]
}

pub(super) fn model_validation_label(rule: &crate::ModelValidationRule) -> &'static str {
    match rule {
        crate::ModelValidationRule::FieldsRequiredTogether { .. } => "联合必填",
        crate::ModelValidationRule::AtLeastOneRequired { .. } => "至少一个必填",
        crate::ModelValidationRule::RequiredWhenPresent { .. } => "条件必填",
    }
}
