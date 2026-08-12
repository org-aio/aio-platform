use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum QueryConditionDraft {
    Field {
        field_id: String,
        operator: String,
        parameter: String,
    },
    Relation {
        relation_field_id: String,
        target_field_id: String,
        operator: String,
        parameter: String,
    },
}

impl QueryConditionDraft {
    fn from_definition(condition: &crate::QueryCondition) -> Self {
        match condition {
            crate::QueryCondition::Field {
                field_id,
                operator,
                parameter,
            } => Self::Field {
                field_id: field_id.to_string(),
                operator: query_operator_key(*operator).to_owned(),
                parameter: parameter.clone(),
            },
            crate::QueryCondition::Relation {
                relation_field_id,
                target_field_id,
                operator,
                parameter,
            } => Self::Relation {
                relation_field_id: relation_field_id.to_string(),
                target_field_id: target_field_id.to_string(),
                operator: query_operator_key(*operator).to_owned(),
                parameter: parameter.clone(),
            },
        }
    }
}

#[component]
pub(super) fn QueryEditorDialog(
    model_id: SymbolId,
    query_count: usize,
    fields: Vec<FieldDefinition>,
    all_models: Vec<ModelDefinition>,
    query: Option<crate::ModelQueryDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let editing = query.is_some();
    let query_id = query.as_ref().map_or_else(SymbolId::new, |query| query.id);
    let stable_name = query
        .as_ref()
        .map(|query| query.name.clone())
        .unwrap_or_default();
    let initial_title = query
        .as_ref()
        .map(|query| query.title.clone())
        .unwrap_or_default();
    let initial_conjunction = query
        .as_ref()
        .map(|query| match query.conjunction {
            crate::QueryConjunction::All => "all".to_owned(),
            crate::QueryConjunction::Any => "any".to_owned(),
        })
        .unwrap_or_else(|| "all".to_owned());
    let initial_conditions = query
        .as_ref()
        .map(|query| {
            query
                .conditions
                .iter()
                .map(QueryConditionDraft::from_definition)
                .collect::<Vec<_>>()
        })
        .filter(|conditions| !conditions.is_empty())
        .unwrap_or_else(|| {
            vec![QueryConditionDraft::Field {
                field_id: String::new(),
                operator: "contains".to_owned(),
                parameter: String::new(),
            }]
        });
    let mut title = use_signal(move || initial_title);
    let mut conjunction = use_signal(move || initial_conjunction);
    let mut conditions = use_signal(move || initial_conditions);
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-query-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    editor.set(None);
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑命名查询" } else { "新建命名查询" } }
                    DialogDescription { "查询条件以结构化字段和命名参数保存" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭查询编辑",
                    aria_label: "关闭查询编辑",
                    onclick: move |_| editor.set(None),
                    icons::X { class: "size-4" }
                }
            }
            form {
                class: "aio-definition-dialog__form aio-query-dialog__form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let next_title = title().trim().to_owned();
                    if next_title.is_empty() {
                        status.set(Some("查询标题不能为空".to_owned()));
                        return;
                    }
                    let next_name = if editing {
                        stable_name.clone()
                    } else {
                        identifier_from_title(&next_title)
                    };
                    if next_name.is_empty() {
                        status.set(Some("查询标题无法生成有效标识，请包含中文、字母或数字".to_owned()));
                        return;
                    }
                    let next_conditions = conditions()
                        .iter()
                        .map(|condition| {
                            query_condition_from_draft(condition, &fields, &all_models)
                        })
                        .collect::<Result<Vec<_>, _>>();
                    let next_conditions = match next_conditions {
                        Ok(conditions) if !conditions.is_empty() => conditions,
                        Ok(_) => {
                            status.set(Some("查询至少需要一个条件".to_owned()));
                            return;
                        }
                        Err(error) => {
                            status.set(Some(error));
                            return;
                        }
                    };
                    let definition = crate::ModelQueryDefinition {
                        id: query_id,
                        name: next_name,
                        title: next_title,
                        conjunction: query_conjunction_from_key(&conjunction()),
                        conditions: next_conditions,
                    };
                    let patches = if editing {
                        vec![GraphPatch::SetProperty {
                            target_id: query_id,
                            property: crate::EditableProperty::ModelQuery,
                            value: serde_json::json!(definition),
                        }]
                    } else {
                        vec![GraphPatch::Insert {
                            parent_id: model_id,
                            collection: ChildCollection::ModelQueries,
                            index: query_count,
                            entity: GraphEntity::ModelQuery(definition),
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
                    h3 { "查询定义" }
                    div { class: "aio-definition-dialog__grid aio-definition-dialog__grid--three",
                        label {
                            span { "显示标题" }
                            Input {
                                class: "aio-input",
                                aria_label: "查询显示标题",
                                placeholder: "例如 按部门查询",
                                value: title(),
                                oninput: move |event: FormEvent| title.set(event.value()),
                            }
                        }
                        label {
                            span { "条件关系" }
                            Select {
                                aria_label: "查询条件关系",
                                value: conjunction(),
                                options: vec![
                                    SelectItem::new("all", "全部满足"),
                                    SelectItem::new("any", "任一满足"),
                                ],
                                on_value_change: move |value| conjunction.set(value),
                            }
                        }
                    }
                }
                section { class: "aio-definition-dialog__section aio-query-dialog__conditions",
                    header {
                        h3 { "查询条件" }
                        div {
                            Button {
                                r#type: "button",
                                size: ButtonSize::Sm,
                                variant: ButtonVariant::Outline,
                                onclick: move |_| conditions.with_mut(|items| items.push(QueryConditionDraft::Field {
                                    field_id: String::new(),
                                    operator: "contains".to_owned(),
                                    parameter: String::new(),
                                })),
                                icons::Plus { class: "size-4" }
                                "字段条件"
                            }
                            Button {
                                r#type: "button",
                                size: ButtonSize::Sm,
                                variant: ButtonVariant::Outline,
                                disabled: !fields.iter().any(|field| field.relation.is_some()),
                                onclick: move |_| conditions.with_mut(|items| items.push(QueryConditionDraft::Relation {
                                    relation_field_id: String::new(),
                                    target_field_id: String::new(),
                                    operator: "contains".to_owned(),
                                    parameter: String::new(),
                                })),
                                icons::Plus { class: "size-4" }
                                "关联条件"
                            }
                        }
                    }
                    div { class: "aio-query-dialog__condition-list",
                        for (index, condition) in conditions().iter().cloned().enumerate() {
                            QueryConditionEditorRow {
                                key: "condition-{index}",
                                index,
                                condition,
                                fields: fields.clone(),
                                all_models: all_models.clone(),
                                conditions,
                            }
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
                        if editing { "保存查询" } else { "创建查询" }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn QueryConditionEditorRow(
    index: usize,
    condition: QueryConditionDraft,
    fields: Vec<FieldDefinition>,
    all_models: Vec<ModelDefinition>,
    mut conditions: Signal<Vec<QueryConditionDraft>>,
) -> Element {
    let relation_target_fields = match &condition {
        QueryConditionDraft::Relation {
            relation_field_id, ..
        } => SymbolId::parse(relation_field_id)
            .ok()
            .and_then(|field_id| fields.iter().find(|field| field.id == field_id))
            .and_then(|field| field.relation.as_ref())
            .and_then(|relation| {
                all_models
                    .iter()
                    .find(|model| model.id == relation.target_model_id)
            })
            .map(|model| model.fields.clone())
            .unwrap_or_default(),
        QueryConditionDraft::Field { .. } => Vec::new(),
    };
    rsx! {
        div { class: "aio-query-dialog__condition",
            div { class: "aio-query-dialog__condition-kind",
                Badge { variant: BadgeVariant::Outline,
                    match condition {
                        QueryConditionDraft::Field { .. } => "字段",
                        QueryConditionDraft::Relation { .. } => "关联",
                    }
                }
            }
            match condition.clone() {
                QueryConditionDraft::Field { field_id, operator, .. } => rsx! {
                    label {
                        span { "字段" }
                        Select {
                            aria_label: "查询条件字段 {index}",
                            value: field_id,
                            placement: SelectPlacement::Top,
                            options: std::iter::once(SelectItem::new("", "选择字段"))
                                .chain(fields.iter().map(|field| SelectItem::new(
                                    field.id.to_string(),
                                    format!("{} · {}", field.title, field.name),
                                )))
                                .collect(),
                            on_value_change: move |value| conditions.with_mut(|items| {
                                if let Some(QueryConditionDraft::Field { field_id, parameter, .. }) = items.get_mut(index) {
                                    let next_parameter = fields
                                        .iter()
                                        .find(|field| field.id.to_string() == value)
                                        .map(|field| field.name.clone())
                                        .unwrap_or_default();
                                    *field_id = value;
                                    *parameter = next_parameter;
                                }
                            }),
                        }
                    }
                    label {
                        span { "匹配" }
                        Select {
                            aria_label: "查询条件操作符 {index}",
                            value: operator,
                            placement: SelectPlacement::Top,
                            options: query_operator_select_items(),
                            on_value_change: move |value| conditions.with_mut(|items| {
                                if let Some(QueryConditionDraft::Field { operator, .. }) = items.get_mut(index) {
                                    *operator = value;
                                }
                            }),
                        }
                    }
                },
                QueryConditionDraft::Relation { relation_field_id, target_field_id, operator, .. } => rsx! {
                    label {
                        span { "关联字段" }
                        Select {
                            aria_label: "关联查询字段 {index}",
                            value: relation_field_id,
                            placement: SelectPlacement::Top,
                            options: std::iter::once(SelectItem::new("", "选择关系"))
                                .chain(fields.iter().filter(|field| field.relation.is_some()).map(|field| SelectItem::new(
                                    field.id.to_string(),
                                    format!("{} · {}", field.title, field.name),
                                )))
                                .collect(),
                            on_value_change: move |value| conditions.with_mut(|items| {
                                if let Some(QueryConditionDraft::Relation {
                                    relation_field_id,
                                    target_field_id,
                                    parameter,
                                    ..
                                }) = items.get_mut(index) {
                                    *relation_field_id = value;
                                    target_field_id.clear();
                                    parameter.clear();
                                }
                            }),
                        }
                    }
                    label {
                        span { "对端字段" }
                        Select {
                            aria_label: "关联查询对端字段 {index}",
                            value: target_field_id,
                            placement: SelectPlacement::Top,
                            options: std::iter::once(SelectItem::new("", "选择字段"))
                                .chain(relation_target_fields.iter().map(|field| SelectItem::new(
                                    field.id.to_string(),
                                    format!("{} · {}", field.title, field.name),
                                )))
                                .collect(),
                            on_value_change: move |value| conditions.with_mut(|items| {
                                if let Some(QueryConditionDraft::Relation {
                                    relation_field_id,
                                    target_field_id,
                                    parameter,
                                    ..
                                }) = items.get_mut(index) {
                                    let relation_name = SymbolId::parse(relation_field_id)
                                        .ok()
                                        .and_then(|field_id| fields.iter().find(|field| field.id == field_id))
                                        .map(|field| field.name.as_str());
                                    let target_name = relation_target_fields
                                        .iter()
                                        .find(|field| field.id.to_string() == value)
                                        .map(|field| field.name.as_str());
                                    *target_field_id = value;
                                    *parameter = match (relation_name, target_name) {
                                        (Some(relation), Some(target)) => format!("{relation}_{target}"),
                                        _ => String::new(),
                                    };
                                }
                            }),
                        }
                    }
                    label {
                        span { "匹配" }
                        Select {
                            aria_label: "关联查询操作符 {index}",
                            value: operator,
                            placement: SelectPlacement::Top,
                            options: query_operator_select_items(),
                            on_value_change: move |value| conditions.with_mut(|items| {
                                if let Some(QueryConditionDraft::Relation { operator, .. }) = items.get_mut(index) {
                                    *operator = value;
                                }
                            }),
                        }
                    }
                },
            }
            Button {
                r#type: "button",
                size: ButtonSize::IconSm,
                variant: ButtonVariant::Ghost,
                title: "移除查询条件",
                aria_label: "移除查询条件 {index}",
                disabled: conditions().len() == 1,
                onclick: move |_| conditions.with_mut(|items| {
                    if items.len() > 1 {
                        items.remove(index);
                    }
                }),
                icons::Trash2 { class: "size-4" }
            }
        }
    }
}

pub(super) fn query_condition_from_draft(
    condition: &QueryConditionDraft,
    fields: &[FieldDefinition],
    all_models: &[ModelDefinition],
) -> Result<crate::QueryCondition, String> {
    match condition {
        QueryConditionDraft::Field {
            field_id,
            operator,
            parameter,
        } => {
            let field_id = SymbolId::parse(field_id).map_err(|_| "请选择查询字段".to_owned())?;
            if !fields.iter().any(|field| field.id == field_id) {
                return Err("查询字段不属于当前模型".to_owned());
            }
            let parameter = parameter.trim();
            let parameter = if parameter.is_empty() {
                fields
                    .iter()
                    .find(|field| field.id == field_id)
                    .map(|field| field.name.clone())
                    .unwrap_or_default()
            } else {
                parameter.to_owned()
            };
            if parameter.is_empty() {
                return Err("查询参数名不能为空".to_owned());
            }
            Ok(crate::QueryCondition::Field {
                field_id,
                operator: query_operator_from_key(operator),
                parameter,
            })
        }
        QueryConditionDraft::Relation {
            relation_field_id,
            target_field_id,
            operator,
            parameter,
        } => {
            let relation_field_id =
                SymbolId::parse(relation_field_id).map_err(|_| "请选择关联字段".to_owned())?;
            let target_field_id =
                SymbolId::parse(target_field_id).map_err(|_| "请选择关联模型字段".to_owned())?;
            let relation = fields
                .iter()
                .find(|field| field.id == relation_field_id)
                .and_then(|field| field.relation.as_ref())
                .ok_or_else(|| "所选字段尚未配置关系".to_owned())?;
            let target_model = all_models
                .iter()
                .find(|model| model.id == relation.target_model_id)
                .ok_or_else(|| "关联模型不存在".to_owned())?;
            if !target_model
                .fields
                .iter()
                .any(|field| field.id == target_field_id)
            {
                return Err("关联查询字段不属于对端模型".to_owned());
            }
            let parameter = parameter.trim();
            let parameter = if parameter.is_empty() {
                let relation_name = fields
                    .iter()
                    .find(|field| field.id == relation_field_id)
                    .map(|field| field.name.as_str())
                    .unwrap_or_default();
                let target_name = target_model
                    .fields
                    .iter()
                    .find(|field| field.id == target_field_id)
                    .map(|field| field.name.as_str())
                    .unwrap_or_default();
                format!("{relation_name}_{target_name}")
            } else {
                parameter.to_owned()
            };
            if parameter.is_empty() {
                return Err("关联查询参数名不能为空".to_owned());
            }
            Ok(crate::QueryCondition::Relation {
                relation_field_id,
                target_field_id,
                operator: query_operator_from_key(operator),
                parameter,
            })
        }
    }
}

pub(super) fn query_operator_key(operator: crate::QueryOperator) -> &'static str {
    match operator {
        crate::QueryOperator::Equals => "equals",
        crate::QueryOperator::NotEquals => "not_equals",
        crate::QueryOperator::Contains => "contains",
        crate::QueryOperator::StartsWith => "starts_with",
        crate::QueryOperator::EndsWith => "ends_with",
        crate::QueryOperator::GreaterThan => "greater_than",
        crate::QueryOperator::GreaterOrEqual => "greater_or_equal",
        crate::QueryOperator::LessThan => "less_than",
        crate::QueryOperator::LessOrEqual => "less_or_equal",
    }
}
