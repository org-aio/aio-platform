use super::*;

#[component]
pub(super) fn IndexEditorDialog(
    model_id: SymbolId,
    index_count: usize,
    fields: Vec<FieldDefinition>,
    index: Option<ModelIndexDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let editing = index.is_some();
    let index_id = index.as_ref().map_or_else(SymbolId::new, |index| index.id);
    let initial_fields = index
        .as_ref()
        .map(|index| index.fields.iter().copied().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let initial_unique = index.as_ref().is_some_and(|index| index.unique);
    let mut selected_fields = use_signal(move || initial_fields);
    let mut unique = use_signal(move || initial_unique);
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-index-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    editor.set(None);
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑索引" } else { "新建索引" } }
                    DialogDescription { "按声明顺序组合字段，可选唯一约束" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭索引编辑",
                    aria_label: "关闭索引编辑",
                    onclick: move |_| editor.set(None),
                    icons::X { class: "size-4" }
                }
            }
            form {
                class: "aio-definition-dialog__form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let selected = selected_fields();
                    let ordered_fields = fields
                        .iter()
                        .filter(|field| selected.contains(&field.id))
                        .map(|field| field.id)
                        .collect::<Vec<_>>();
                    if ordered_fields.is_empty() {
                        status.set(Some("索引至少需要一个字段".to_owned()));
                        return;
                    }
                    let patches = if editing {
                        vec![
                            GraphPatch::SetProperty {
                                target_id: index_id,
                                property: crate::EditableProperty::ModelIndexFields,
                                value: serde_json::json!(ordered_fields),
                            },
                            GraphPatch::SetProperty {
                                target_id: index_id,
                                property: crate::EditableProperty::ModelIndexUnique,
                                value: serde_json::json!(unique()),
                            },
                        ]
                    } else {
                        let definition = ModelIndexDefinition {
                            id: index_id,
                            fields: ordered_fields,
                            unique: unique(),
                        };
                        vec![GraphPatch::Insert {
                            parent_id: model_id,
                            collection: ChildCollection::ModelIndexes,
                            index: index_count,
                            entity: Box::new(GraphEntity::ModelIndex(definition)),
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
                section { class: "aio-definition-dialog__section",
                    h3 { "索引字段" }
                    div { class: "aio-definition-dialog__choice-list",
                        for field in &fields {
                            label {
                                Checkbox {
                                    aria_label: "索引字段 {field.title}",
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
                label { class: "aio-definition-dialog__checkbox-field",
                    Checkbox {
                        aria_label: "唯一索引",
                        checked: Some(checkbox_state(unique())),
                        on_checked_change: move |checked| unique.set(checkbox_is_checked(checked)),
                    }
                    span { "组合值必须唯一" }
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
                        if editing { "保存索引" } else { "创建索引" }
                    }
                }
            }
        }
    }
}

pub(super) fn editable_value_type_options(current: &ValueType) -> Vec<SelectItem> {
    let mut options = vec![
        SelectItem::new("text", "文本"),
        SelectItem::new("integer", "整数"),
        SelectItem::new("decimal", "小数"),
        SelectItem::new("boolean", "布尔"),
        SelectItem::new("timestamp_ms", "时间"),
        SelectItem::new("file", "文件"),
        SelectItem::new("any", "任意结构"),
    ];
    if editable_value_type_key(current) == "preserve" {
        options.push(SelectItem::new(
            "preserve",
            format!("{}（保持定义）", value_type_label(current)),
        ));
    }
    options
}

pub(super) fn editable_value_type_key(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Text => "text",
        ValueType::Integer => "integer",
        ValueType::Decimal => "decimal",
        ValueType::Boolean => "boolean",
        ValueType::TimestampMs => "timestamp_ms",
        ValueType::File => "file",
        ValueType::Any => "any",
        ValueType::Null
        | ValueType::Object { .. }
        | ValueType::List { .. }
        | ValueType::Optional { .. } => "preserve",
    }
}

pub(super) fn editable_value_type_from_key(key: &str, current: &ValueType) -> ValueType {
    if key == "preserve" {
        current.clone()
    } else {
        value_type_from_key(key)
    }
}

pub(super) fn relation_kind_key(kind: crate::RelationKind) -> &'static str {
    match kind {
        crate::RelationKind::OneToOne => "one_to_one",
        crate::RelationKind::ManyToOne => "many_to_one",
        crate::RelationKind::OneToMany => "one_to_many",
        crate::RelationKind::ManyToMany => "many_to_many",
    }
}

pub(super) fn relation_kind_from_key(value: &str) -> crate::RelationKind {
    match value {
        "one_to_one" => crate::RelationKind::OneToOne,
        "one_to_many" => crate::RelationKind::OneToMany,
        "many_to_many" => crate::RelationKind::ManyToMany,
        _ => crate::RelationKind::ManyToOne,
    }
}

pub(super) fn relation_value_type(
    kind: crate::RelationKind,
    target_model_id: SymbolId,
) -> ValueType {
    let value = ValueType::Object {
        model_id: target_model_id,
    };
    if kind.is_collection() {
        ValueType::List {
            item: Box::new(value),
        }
    } else {
        value
    }
}

pub(super) fn value_type_from_key(key: &str) -> ValueType {
    match key {
        "integer" => ValueType::Integer,
        "decimal" => ValueType::Decimal,
        "boolean" => ValueType::Boolean,
        "timestamp_ms" => ValueType::TimestampMs,
        "file" => ValueType::File,
        "any" => ValueType::Any,
        _ => ValueType::Text,
    }
}

pub(super) fn value_type_label(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Any => "任意结构",
        ValueType::Null => "空值",
        ValueType::Boolean => "布尔",
        ValueType::Integer => "整数",
        ValueType::Decimal => "小数",
        ValueType::Text => "文本",
        ValueType::TimestampMs => "时间",
        ValueType::File => "文件",
        ValueType::Object { .. } => "对象",
        ValueType::List { .. } => "列表",
        ValueType::Optional { .. } => "可选",
    }
}
