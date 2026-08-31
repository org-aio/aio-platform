use super::*;

#[component]
pub(super) fn RelationEditorDialog(
    field: FieldDefinition,
    source_model: ModelDefinition,
    all_models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let field_id = field.id;
    let initial_kind = field
        .relation
        .as_ref()
        .map(|relation| relation_kind_key(relation.kind).to_owned())
        .unwrap_or_else(|| "many_to_one".to_owned());
    let initial_model = field
        .relation
        .as_ref()
        .map(|relation| relation.target_model_id.to_string())
        .unwrap_or_default();
    let initial_field = field
        .relation
        .as_ref()
        .map(|relation| relation.target_field_id.to_string())
        .unwrap_or_default();
    let mut kind = use_signal(move || initial_kind);
    let mut target_model = use_signal(move || initial_model);
    let mut target_field = use_signal(move || initial_field);
    let mut confirming_unlink = use_signal(|| false);
    let selected_target_model = SymbolId::parse(&target_model()).ok();
    let target_fields = selected_target_model
        .and_then(|model_id| all_models.iter().find(|model| model.id == model_id))
        .map(|model| model.fields.clone())
        .unwrap_or_default();
    let has_relation = field.relation.is_some();
    let previous_relation = field.relation.clone();
    let save_api = api_base_url.clone();
    let save_program_id = program_id.clone();
    let unlink_api = api_base_url;
    let unlink_program_id = program_id;
    rsx! {
        if !confirming_unlink() {
            Dialog {
                class: "aio-definition-dialog aio-relation-dialog",
                open: true,
                on_open_change: move |open: bool| {
                    if !open {
                        editor.set(None);
                    }
                },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "配置字段关系" }
                    DialogDescription { "{source_model.title}.{field.title}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭字段关系配置",
                    aria_label: "关闭字段关系配置",
                    onclick: move |_| editor.set(None),
                    icons::X { class: "size-4" }
                }
            }
            form {
                class: "aio-definition-dialog__form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let Ok(target_model_id) = SymbolId::parse(&target_model()) else {
                        status.set(Some("请选择关联模型".to_owned()));
                        return;
                    };
                    let Ok(target_field_id) = SymbolId::parse(&target_field()) else {
                        status.set(Some("请选择对端字段".to_owned()));
                        return;
                    };
                    let Some(target) = all_models.iter().find(|model| model.id == target_model_id) else {
                        status.set(Some("关联模型不存在".to_owned()));
                        return;
                    };
                    let Some(other_field) = target.fields.iter().find(|candidate| candidate.id == target_field_id) else {
                        status.set(Some("对端字段不属于关联模型".to_owned()));
                        return;
                    };
                    let relation_kind = relation_kind_from_key(&kind());
                    if target_model_id == source_model.id
                        && target_field_id == field_id
                        && relation_kind != relation_kind.opposite()
                    {
                        status.set(Some("同一字段自关联只能使用对称基数".to_owned()));
                        return;
                    }
                    if other_field.relation.as_ref().is_some_and(|relation| {
                        relation.target_model_id != source_model.id
                            || relation.target_field_id != field_id
                    }) {
                        status.set(Some("对端字段已关联到其他字段，请先解除原关联".to_owned()));
                        return;
                    }
                    let source_relation = crate::FieldRelation {
                        kind: relation_kind,
                        target_model_id,
                        target_field_id,
                    };
                    let target_relation = crate::FieldRelation {
                        kind: relation_kind.opposite(),
                        target_model_id: source_model.id,
                        target_field_id: field_id,
                    };
                    let mut patches = vec![
                        GraphPatch::SetProperty {
                            target_id: field_id,
                            property: crate::EditableProperty::FieldRelation,
                            value: serde_json::json!(source_relation),
                        },
                        GraphPatch::SetProperty {
                            target_id: field_id,
                            property: crate::EditableProperty::FieldValueType,
                            value: serde_json::json!(relation_value_type(relation_kind, target_model_id)),
                        },
                    ];
                    if let Some(previous) = &previous_relation
                        && (previous.target_model_id != target_model_id
                            || previous.target_field_id != target_field_id)
                    {
                        patches.extend(relation_target_clear_patches(previous.target_field_id));
                    }
                    if target_field_id != field_id {
                        patches.extend([
                            GraphPatch::SetProperty {
                                target_id: target_field_id,
                                property: crate::EditableProperty::FieldRelation,
                                value: serde_json::json!(target_relation),
                            },
                            GraphPatch::SetProperty {
                                target_id: target_field_id,
                                property: crate::EditableProperty::FieldValueType,
                                value: serde_json::json!(relation_value_type(
                                    relation_kind.opposite(),
                                    source_model.id,
                                )),
                            },
                        ]);
                    }
                    submit_patches(
                        save_api.clone(),
                        save_program_id.clone(),
                        version,
                        patches,
                        generation,
                        status,
                    );
                    editor.set(None);
                },
                label {
                    span { "关联基数" }
                    Select {
                        class: "aio-input",
                        aria_label: "关联基数",
                        value: kind(),
                        options: relation_kind_select_items(),
                        on_value_change: move |value: String| kind.set(value),
                    }
                }
                label {
                    span { "关联模型" }
                    Select {
                        class: "aio-input",
                        aria_label: "关联模型",
                        value: target_model(),
                        options: std::iter::once(SelectItem::new("", "选择模型"))
                            .chain(all_models.iter().map(|model| SelectItem::new(
                                model.id.to_string(),
                                format!("{} · {}", model.title, model.name),
                            )))
                            .collect(),
                        on_value_change: move |value: String| {
                            target_model.set(value);
                            target_field.set(String::new());
                        },
                    }
                }
                label {
                    span { "对端字段" }
                    Select {
                        class: "aio-input",
                        aria_label: "关联对端字段",
                        value: target_field(),
                        options: std::iter::once(SelectItem::new("", "选择字段"))
                            .chain(target_fields.iter().map(|field| SelectItem::new(
                                field.id.to_string(),
                                format!("{} · {}", field.title, field.name),
                            )))
                            .collect(),
                        on_value_change: move |value: String| target_field.set(value),
                    }
                }
                footer { class: "aio-definition-dialog__actions aio-definition-dialog__actions--split",
                    div {
                        if has_relation {
                            Button {
                                r#type: "button",
                                variant: ButtonVariant::Destructive,
                                onclick: move |_| confirming_unlink.set(true),
                                icons::Unlink { class: "size-4" }
                                "解除关系"
                            }
                        }
                    }
                    div {
                        Button {
                            r#type: "button",
                            variant: ButtonVariant::Ghost,
                            onclick: move |_| editor.set(None),
                            "取消"
                        }
                        Button {
                            r#type: "submit",
                            icons::Save { class: "size-4" }
                            "保存关系"
                        }
                    }
                }
            }
            }
        }
        if confirming_unlink() {
            Dialog {
                class: "aio-definition-confirm-dialog",
                open: true,
                on_open_change: move |open: bool| {
                    if !open {
                        confirming_unlink.set(false);
                    }
                },
                DialogTitle { "解除字段关系" }
                DialogDescription { "确认解除 {source_model.title}.{field.title} 的双向关系？" }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| confirming_unlink.set(false),
                        "取消"
                    }
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Destructive,
                        onclick: move |_| {
                            let patches = relation_unlink_patches(&field, source_model.id);
                            submit_patches(
                                unlink_api.clone(),
                                unlink_program_id.clone(),
                                version,
                                patches,
                                generation,
                                status,
                            );
                            confirming_unlink.set(false);
                            editor.set(None);
                        },
                        icons::Unlink { class: "size-4" }
                        "解除"
                    }
                }
            }
        }
    }
}

pub(super) fn relation_target_clear_patches(target_field_id: SymbolId) -> [GraphPatch; 2] {
    [
        GraphPatch::SetProperty {
            target_id: target_field_id,
            property: crate::EditableProperty::FieldRelation,
            value: serde_json::Value::Null,
        },
        GraphPatch::SetProperty {
            target_id: target_field_id,
            property: crate::EditableProperty::FieldValueType,
            value: serde_json::json!(ValueType::Text),
        },
    ]
}

pub(super) fn relation_unlink_patches(
    field: &FieldDefinition,
    source_model_id: SymbolId,
) -> Vec<GraphPatch> {
    let mut patches = Vec::from(relation_target_clear_patches(field.id));
    if let Some(relation) = &field.relation
        && (relation.target_model_id != source_model_id || relation.target_field_id != field.id)
    {
        patches.extend(relation_target_clear_patches(relation.target_field_id));
    }
    patches
}

pub(super) fn optional_u32(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

pub(super) fn parse_optional_u32(value: &str) -> Option<u32> {
    value.trim().parse().ok()
}

pub(super) fn optional_f64(value: Option<f64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

pub(super) fn parse_optional_f64(value: &str) -> Option<f64> {
    value.trim().parse().ok()
}

pub(super) fn editable_default_value(value: &str) -> Option<Value> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned())))
    }
}

pub(super) fn non_empty_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
