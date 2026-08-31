use super::*;

#[component]
pub(super) fn FieldEditorDialog(
    model_id: SymbolId,
    field_count: usize,
    field: Option<FieldDefinition>,
    all_models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let editing = field.is_some();
    let initial_field = field.unwrap_or_else(|| FieldDefinition {
        id: SymbolId::new(),
        name: String::new(),
        title: String::new(),
        value_type: ValueType::Text,
        state: DefinitionState::Known,
        required: false,
        options: crate::FieldOptions::default(),
        relation: None,
    });
    let initial_default_value = initial_field
        .options
        .default_value
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    let stable_name = initial_field.name.clone();
    let previous_relation = initial_field.relation.clone();
    let initial_type = if initial_field.relation.is_some() {
        "relation".to_owned()
    } else {
        editable_value_type_key(&initial_field.value_type).to_owned()
    };
    let initial_relation_kind = initial_field
        .relation
        .as_ref()
        .map(|relation| relation_kind_key(relation.kind).to_owned())
        .unwrap_or_else(|| "many_to_one".to_owned());
    let initial_target_model = initial_field
        .relation
        .as_ref()
        .map(|relation| relation.target_model_id.to_string())
        .unwrap_or_default();
    let initial_target_field = initial_field
        .relation
        .as_ref()
        .map(|relation| relation.target_field_id.to_string())
        .unwrap_or_default();
    let mut draft = use_signal(move || initial_field);
    let mut default_value = use_signal(move || initial_default_value);
    let mut field_type = use_signal(move || initial_type);
    let mut relation_kind = use_signal(move || initial_relation_kind);
    let mut target_model = use_signal(move || initial_target_model);
    let mut target_field = use_signal(move || initial_target_field);
    let relation_selected = field_type() == "relation";
    let target_fields = SymbolId::parse(&target_model())
        .ok()
        .and_then(|target_model_id| all_models.iter().find(|model| model.id == target_model_id))
        .map(|model| model.fields.clone())
        .unwrap_or_default();
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-field-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    editor.set(None);
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑字段" } else { "新建字段" } }
                    DialogDescription {
                        if relation_selected {
                            "选择关联模型、基数与对端字段"
                        } else {
                            "定义字段结构、页面行为与校验约束"
                        }
                    }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭字段编辑",
                    aria_label: "关闭字段编辑",
                    onclick: move |_| editor.set(None),
                    icons::X { class: "size-4" }
                }
            }
            form {
                class: "aio-definition-dialog__form aio-field-dialog__form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let mut next = draft();
                    next.title = next.title.trim().to_owned();
                    if next.title.is_empty() {
                        status.set(Some("字段标题不能为空".to_owned()));
                        return;
                    }
                    next.name = if editing {
                        stable_name.clone()
                    } else {
                        identifier_from_title(&next.title)
                    };
                    if next.name.is_empty() {
                        status.set(Some("字段标题无法生成有效标识，请包含中文、字母或数字".to_owned()));
                        return;
                    }
                    if next.name == "id" {
                        status.set(Some("id 是系统主键字段，请直接选择主键生成策略".to_owned()));
                        return;
                    }
                    let relation = if relation_selected {
                        let Ok(target_model_id) = SymbolId::parse(&target_model()) else {
                            status.set(Some("请选择关联模型".to_owned()));
                            return;
                        };
                        let Ok(target_field_id) = SymbolId::parse(&target_field()) else {
                            status.set(Some("请选择对端字段".to_owned()));
                            return;
                        };
                        let Some(target) = all_models
                            .iter()
                            .find(|model| model.id == target_model_id)
                        else {
                            status.set(Some("关联模型不存在".to_owned()));
                            return;
                        };
                        let Some(other_field) = target
                            .fields
                            .iter()
                            .find(|candidate| candidate.id == target_field_id)
                        else {
                            status.set(Some("对端字段不属于关联模型".to_owned()));
                            return;
                        };
                        let kind = relation_kind_from_key(&relation_kind());
                        if target_model_id == model_id
                            && target_field_id == next.id
                            && kind != kind.opposite()
                        {
                            status.set(Some("同一字段自关联只能使用对称基数".to_owned()));
                            return;
                        }
                        if other_field.relation.as_ref().is_some_and(|relation| {
                            relation.target_model_id != model_id
                                || relation.target_field_id != next.id
                        }) {
                            status.set(Some("对端字段已关联到其他字段，请先解除原关联".to_owned()));
                            return;
                        }
                        next.value_type = relation_value_type(kind, target_model_id);
                        Some(crate::FieldRelation {
                            kind,
                            target_model_id,
                            target_field_id,
                        })
                    } else {
                        next.value_type = editable_value_type_from_key(
                            &field_type(),
                            &next.value_type,
                        );
                        None
                    };
                    next.relation = relation.clone();
                    next.options.default_value = editable_default_value(&default_value());
                    let field_id = next.id;
                    let mut patches = if editing {
                        vec![
                            GraphPatch::Rename {
                                target_id: next.id,
                                name: next.name.clone(),
                                title: Some(next.title.clone()),
                            },
                            GraphPatch::SetProperty {
                                target_id: next.id,
                                property: crate::EditableProperty::FieldValueType,
                                value: serde_json::json!(next.value_type),
                            },
                            GraphPatch::SetProperty {
                                target_id: next.id,
                                property: crate::EditableProperty::FieldRequired,
                                value: serde_json::json!(next.required),
                            },
                            GraphPatch::SetProperty {
                                target_id: next.id,
                                property: crate::EditableProperty::FieldOptions,
                                value: serde_json::json!(next.options),
                            },
                            GraphPatch::SetProperty {
                                target_id: next.id,
                                property: crate::EditableProperty::FieldRelation,
                                value: serde_json::json!(next.relation),
                            },
                        ]
                    } else {
                        vec![GraphPatch::Insert {
                            parent_id: model_id,
                            collection: ChildCollection::Fields,
                            index: field_count,
                            entity: Box::new(GraphEntity::Field(next)),
                        }]
                    };
                    if let Some(previous) = &previous_relation
                        && relation.as_ref().is_none_or(|current| {
                            current.target_model_id != previous.target_model_id
                                || current.target_field_id != previous.target_field_id
                        })
                    {
                        patches.extend(relation_target_clear_patches(previous.target_field_id));
                    }
                    if let Some(relation) = relation
                        && relation.target_field_id != field_id
                    {
                        let target_relation = crate::FieldRelation {
                            kind: relation.kind.opposite(),
                            target_model_id: model_id,
                            target_field_id: field_id,
                        };
                        patches.extend([
                            GraphPatch::SetProperty {
                                target_id: relation.target_field_id,
                                property: crate::EditableProperty::FieldRelation,
                                value: serde_json::json!(target_relation),
                            },
                            GraphPatch::SetProperty {
                                target_id: relation.target_field_id,
                                property: crate::EditableProperty::FieldValueType,
                                value: serde_json::json!(relation_value_type(
                                    relation.kind.opposite(),
                                    model_id,
                                )),
                            },
                        ]);
                    }
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
                    h3 { "基础定义" }
                    div { class: "aio-definition-dialog__grid",
                        label {
                            span { "显示标题" }
                            Input {
                                class: "aio-input",
                                aria_label: "字段显示标题",
                                placeholder: "例如 状态",
                                value: draft().title.clone(),
                                oninput: move |event: FormEvent| {
                                    draft.with_mut(|field| field.title = event.value());
                                },
                            }
                        }
                        label {
                            span { "字段类型" }
                            Select {
                                aria_label: "字段类型",
                                value: field_type(),
                                options: field_value_type_select_items(&draft().value_type),
                                on_value_change: move |value: String| {
                                    field_type.set(value);
                                },
                            }
                        }
                        label { class: "aio-definition-dialog__checkbox-field",
                            Checkbox {
                                aria_label: "字段必填",
                                checked: Some(checkbox_state(draft().required)),
                                on_checked_change: move |checked| {
                                    draft.with_mut(|field| {
                                        field.required = checkbox_is_checked(checked);
                                    });
                                },
                            }
                            span { "必填字段" }
                        }
                    }
                    if relation_selected {
                        label {
                            span { "关联基数" }
                            Select {
                                aria_label: "关联基数",
                                value: relation_kind(),
                                options: relation_kind_select_items(),
                                on_value_change: move |value: String| relation_kind.set(value),
                            }
                        }
                        label {
                            span { "关联模型" }
                            Select {
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
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "页面与数据能力" }
                    div { class: "aio-field-dialog__toggles",
                        label { Checkbox {
                            aria_label: "列表显示",
                            checked: Some(checkbox_state(draft().options.list_visible)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.list_visible = checkbox_is_checked(checked)),
                        } span { "列表显示" } }
                        label { Checkbox {
                            aria_label: "详情显示",
                            checked: Some(checkbox_state(draft().options.detail_visible)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.detail_visible = checkbox_is_checked(checked)),
                        } span { "详情显示" } }
                        label { Checkbox {
                            aria_label: "表单显示",
                            checked: Some(checkbox_state(draft().options.form_visible)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.form_visible = checkbox_is_checked(checked)),
                        } span { "表单显示" } }
                        label { Checkbox {
                            aria_label: "表单可编辑",
                            checked: Some(checkbox_state(draft().options.form_editable)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.form_editable = checkbox_is_checked(checked)),
                        } span { "表单可编辑" } }
                        label { Checkbox {
                            aria_label: "允许查询",
                            checked: Some(checkbox_state(draft().options.filterable)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.filterable = checkbox_is_checked(checked)),
                        } span { "允许查询" } }
                        label { Checkbox {
                            aria_label: "允许排序",
                            checked: Some(checkbox_state(draft().options.sortable)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.sortable = checkbox_is_checked(checked)),
                        } span { "允许排序" } }
                        label { Checkbox {
                            aria_label: "唯一约束",
                            checked: Some(checkbox_state(draft().options.unique)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.unique = checkbox_is_checked(checked)),
                        } span { "唯一约束" } }
                        label { Checkbox {
                            aria_label: "Excel 导入",
                            checked: Some(checkbox_state(draft().options.excel_import)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.excel_import = checkbox_is_checked(checked)),
                        } span { "Excel 导入" } }
                        label { Checkbox {
                            aria_label: "Excel 导出",
                            checked: Some(checkbox_state(draft().options.excel_export)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.excel_export = checkbox_is_checked(checked)),
                        } span { "Excel 导出" } }
                        label { Checkbox {
                            aria_label: "AI 结构化提取",
                            checked: Some(checkbox_state(draft().options.ai_extract)),
                            on_checked_change: move |checked| draft.with_mut(|field| field.options.ai_extract = checkbox_is_checked(checked)),
                        } span { "AI 结构化提取" } }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "默认值与提示" }
                    div { class: "aio-definition-dialog__grid",
                        label {
                            span { "默认值" }
                            Input {
                                class: "aio-input",
                                aria_label: "字段默认值",
                                placeholder: "JSON 或文本",
                                value: default_value(),
                                oninput: move |event: FormEvent| default_value.set(event.value()),
                            }
                        }
                        label {
                            span { "占位提示" }
                            Input {
                                class: "aio-input",
                                aria_label: "字段占位提示",
                                value: draft().options.placeholder.clone().unwrap_or_default(),
                                oninput: move |event: FormEvent| draft.with_mut(|field| field.options.placeholder = non_empty_text(&event.value())),
                            }
                        }
                        label { class: "aio-definition-dialog__wide-field",
                            span { "帮助文本" }
                            Textarea {
                                class: "aio-input",
                                aria_label: "字段帮助文本",
                                rows: "2",
                                value: draft().options.help_text.clone().unwrap_or_default(),
                                oninput: move |event: FormEvent| draft.with_mut(|field| field.options.help_text = non_empty_text(&event.value())),
                            }
                        }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "单字段校验" }
                    div { class: "aio-field-dialog__validation-grid",
                        label { span { "最小长度" } Input {
                            r#type: "number", min: "0", class: "aio-input", aria_label: "最小文本长度",
                            value: optional_u32(draft().options.validation.min_length),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.min_length = parse_optional_u32(&event.value())),
                        } }
                        label { span { "最大长度" } Input {
                            r#type: "number", min: "0", class: "aio-input", aria_label: "最大文本长度",
                            value: optional_u32(draft().options.validation.max_length),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.max_length = parse_optional_u32(&event.value())),
                        } }
                        label { span { "最小数值" } Input {
                            class: "aio-input", aria_label: "最小数值",
                            value: optional_f64(draft().options.validation.minimum),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.minimum = parse_optional_f64(&event.value())),
                        } }
                        label { span { "最大数值" } Input {
                            class: "aio-input", aria_label: "最大数值",
                            value: optional_f64(draft().options.validation.maximum),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.maximum = parse_optional_f64(&event.value())),
                        } }
                        label { span { "最少项数" } Input {
                            r#type: "number", min: "0", class: "aio-input", aria_label: "列表最少项数",
                            value: optional_u32(draft().options.validation.min_items),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.min_items = parse_optional_u32(&event.value())),
                        } }
                        label { span { "最多项数" } Input {
                            r#type: "number", min: "0", class: "aio-input", aria_label: "列表最多项数",
                            value: optional_u32(draft().options.validation.max_items),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.max_items = parse_optional_u32(&event.value())),
                        } }
                        label { class: "aio-definition-dialog__wide-field", span { "正则表达式" } Input {
                            class: "aio-input", aria_label: "字段正则表达式",
                            value: draft().options.validation.pattern.clone().unwrap_or_default(),
                            oninput: move |event: FormEvent| draft.with_mut(|field| field.options.validation.pattern = non_empty_text(&event.value())),
                        } }
                        label { class: "aio-definition-dialog__checkbox-field aio-definition-dialog__wide-field",
                            Checkbox {
                                aria_label: "列表元素唯一",
                                checked: Some(checkbox_state(draft().options.validation.unique_items)),
                                on_checked_change: move |checked| draft.with_mut(|field| field.options.validation.unique_items = checkbox_is_checked(checked)),
                            }
                            span { "列表元素不能重复" }
                        }
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
                        if editing { "保存字段" } else { "创建字段" }
                    }
                }
            }
        }
    }
}

pub(super) fn field_value_type_select_items(current: &ValueType) -> Vec<SelectItem> {
    let mut options = vec![
        SelectItem::new("text", "文本"),
        SelectItem::new("integer", "整数"),
        SelectItem::new("decimal", "小数"),
        SelectItem::new("boolean", "布尔"),
        SelectItem::new("timestamp_ms", "时间"),
        SelectItem::new("file", "文件"),
        SelectItem::new("any", "任意结构"),
        SelectItem::new("relation", "关联对象"),
    ];
    if editable_value_type_key(current) == "preserve" {
        options.push(SelectItem::new(
            "preserve",
            format!("{}（保持定义）", value_type_label(current)),
        ));
    }
    options
}

pub(super) fn relation_kind_select_items() -> Vec<SelectItem> {
    vec![
        SelectItem::new("one_to_one", "一对一"),
        SelectItem::new("many_to_one", "多对一"),
        SelectItem::new("one_to_many", "一对多"),
        SelectItem::new("many_to_many", "多对多"),
    ]
}
