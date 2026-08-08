use super::*;

#[component]
pub(super) fn ModelIndexesTable(
    model: ModelDefinition,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let fields_for_cells = model.fields.clone();
    let columns = vec![
        DataTableColumn::leaf("order", "序号")
            .width(72)
            .align(DataTableAlign::Center)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("fields", "索引字段").width(440),
        DataTableColumn::leaf("unique", "唯一索引")
            .width(112)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("actions", "操作")
            .width(96)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ];
    rsx! {
        DataTable::<ModelIndexDefinition> {
            class: "aio-model-data-table",
            aria_label: "模型索引",
            rows: model.indexes.clone(),
            columns,
            max_height: "100%",
            empty_text: "暂无索引".to_owned(),
            row_key: |index: ModelIndexDefinition| index.id.to_string(),
            render_cell: move |cell: DataTableCellContext<ModelIndexDefinition>| {
                model_index_cell(cell, fields_for_cells.clone(), editor, deleting)
            },
        }
    }
}

pub(super) fn model_index_cell(
    cell: DataTableCellContext<ModelIndexDefinition>,
    fields: Vec<FieldDefinition>,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let index = cell.row;
    match cell.column.key.as_str() {
        "order" => rsx! { "{cell.row_index + 1}" },
        "fields" => {
            let labels = index
                .fields
                .iter()
                .map(|field_id| {
                    fields
                        .iter()
                        .find(|field| field.id == *field_id)
                        .map(|field| field.title.clone())
                        .unwrap_or_else(|| "未知字段".to_owned())
                })
                .collect::<Vec<_>>()
                .join(" + ");
            rsx! { strong { "{labels}" } }
        }
        "unique" => rsx! {
            if index.unique {
                Badge { "唯一" }
            } else {
                span { class: "aio-model-table__muted", "普通" }
            }
        },
        "actions" => {
            let index_id = index.id;
            let label = format!("索引 {}", cell.row_index + 1);
            rsx! {
                div { class: "aio-model-table__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "编辑索引",
                        aria_label: "编辑 {label}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editor.set(Some(ModelEditorTarget::EditIndex(index_id)));
                        },
                        icons::Pencil { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "删除索引",
                        aria_label: "删除 {label}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            deleting.set(Some(DefinitionDeleteTarget {
                                id: index_id,
                                kind: "索引",
                                label: label.clone(),
                            }));
                        },
                        icons::Trash2 { class: "size-4" }
                    }
                }
            }
        }
        _ => rsx! { "—" },
    }
}

#[component]
pub(super) fn ModelQueriesTable(
    model: ModelDefinition,
    all_models: Vec<ModelDefinition>,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let fields_for_cells = model.fields.clone();
    let models_for_cells = all_models.clone();
    let columns = vec![
        DataTableColumn::group(
            "identity",
            "命名查询",
            vec![
                DataTableColumn::leaf("title", "显示标题")
                    .width(180)
                    .fixed(DataTableFixed::Left),
                DataTableColumn::leaf("name", "查询标识").width(180),
            ],
        ),
        DataTableColumn::leaf("conjunction", "条件关系")
            .width(112)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("conditions", "查询条件").width(480),
        DataTableColumn::leaf("actions", "操作")
            .width(96)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ];
    rsx! {
        DataTable::<crate::ModelQueryDefinition> {
            class: "aio-model-data-table",
            aria_label: "模型命名查询",
            rows: model.queries.clone(),
            columns,
            max_height: "100%",
            empty_text: "暂无命名查询".to_owned(),
            row_key: |query: crate::ModelQueryDefinition| query.id.to_string(),
            render_cell: move |cell: DataTableCellContext<crate::ModelQueryDefinition>| {
                model_query_cell(
                    cell,
                    fields_for_cells.clone(),
                    models_for_cells.clone(),
                    editor,
                    deleting,
                )
            },
        }
    }
}

pub(super) fn model_query_cell(
    cell: DataTableCellContext<crate::ModelQueryDefinition>,
    fields: Vec<FieldDefinition>,
    all_models: Vec<ModelDefinition>,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let query = cell.row;
    match cell.column.key.as_str() {
        "title" => rsx! { strong { "{query.title}" } },
        "name" => rsx! { code { class: "aio-model-table__code", "{query.name}" } },
        "conjunction" => rsx! {
            Badge { variant: BadgeVariant::Outline,
                if query.conjunction == crate::QueryConjunction::All { "全部满足" } else { "任一满足" }
            }
        },
        "conditions" => {
            let summary = query
                .conditions
                .iter()
                .map(|condition| query_condition_summary(condition, &fields, &all_models))
                .collect::<Vec<_>>()
                .join("；");
            rsx! { span { class: "aio-model-table__summary", "{summary}" } }
        }
        "actions" => {
            let query_id = query.id;
            let query_title = query.title.clone();
            rsx! {
                div { class: "aio-model-table__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "编辑查询",
                        aria_label: "编辑查询 {query.name}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editor.set(Some(ModelEditorTarget::EditQuery(query_id)));
                        },
                        icons::Pencil { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "删除查询",
                        aria_label: "删除查询 {query.name}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            deleting.set(Some(DefinitionDeleteTarget {
                                id: query_id,
                                kind: "查询",
                                label: query_title.clone(),
                            }));
                        },
                        icons::Trash2 { class: "size-4" }
                    }
                }
            }
        }
        _ => rsx! { "—" },
    }
}

pub(super) fn query_condition_summary(
    condition: &crate::QueryCondition,
    fields: &[FieldDefinition],
    all_models: &[ModelDefinition],
) -> String {
    match condition {
        crate::QueryCondition::Field {
            field_id,
            operator,
            parameter,
        } => {
            let title = fields
                .iter()
                .find(|field| field.id == *field_id)
                .map(|field| field.title.as_str())
                .unwrap_or("未知字段");
            format!("{title} {} :{parameter}", query_operator_label(*operator))
        }
        crate::QueryCondition::Relation {
            relation_field_id,
            target_field_id,
            operator,
            parameter,
        } => {
            let relation_field = fields.iter().find(|field| field.id == *relation_field_id);
            let relation_title = relation_field
                .map(|field| field.title.as_str())
                .unwrap_or("未知关系");
            let target_title = relation_field
                .and_then(|field| field.relation.as_ref())
                .and_then(|relation| {
                    all_models
                        .iter()
                        .find(|model| model.id == relation.target_model_id)
                })
                .and_then(|model| {
                    model
                        .fields
                        .iter()
                        .find(|field| field.id == *target_field_id)
                })
                .map(|field| field.title.as_str())
                .unwrap_or("未知字段");
            format!(
                "{relation_title}.{target_title} {} :{parameter}",
                query_operator_label(*operator)
            )
        }
    }
}

pub(super) fn query_operator_label(operator: crate::QueryOperator) -> &'static str {
    match operator {
        crate::QueryOperator::Equals => "等于",
        crate::QueryOperator::NotEquals => "不等于",
        crate::QueryOperator::Contains => "包含",
        crate::QueryOperator::StartsWith => "开头是",
        crate::QueryOperator::EndsWith => "结尾是",
        crate::QueryOperator::GreaterThan => "大于",
        crate::QueryOperator::GreaterOrEqual => "大于等于",
        crate::QueryOperator::LessThan => "小于",
        crate::QueryOperator::LessOrEqual => "小于等于",
    }
}

#[component]
pub(super) fn ModelValidationsTable(
    model: ModelDefinition,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let fields_for_cells = model.fields.clone();
    let columns = vec![
        DataTableColumn::leaf("rule", "规则")
            .width(160)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("fields", "参与字段").width(320),
        DataTableColumn::leaf("message", "失败提示").width(420),
        DataTableColumn::leaf("actions", "操作")
            .width(96)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ];
    rsx! {
        DataTable::<crate::ModelValidationDefinition> {
            class: "aio-model-data-table",
            aria_label: "模型级校验",
            rows: model.validations.clone(),
            columns,
            max_height: "100%",
            empty_text: "暂无模型级校验".to_owned(),
            row_key: |validation: crate::ModelValidationDefinition| validation.id.to_string(),
            render_cell: move |cell: DataTableCellContext<crate::ModelValidationDefinition>| {
                model_validation_cell(cell, fields_for_cells.clone(), editor, deleting)
            },
        }
    }
}

pub(super) fn model_validation_cell(
    cell: DataTableCellContext<crate::ModelValidationDefinition>,
    fields: Vec<FieldDefinition>,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let validation = cell.row;
    match cell.column.key.as_str() {
        "rule" => rsx! {
            Badge { variant: BadgeVariant::Outline, "{model_validation_label(&validation.rule)}" }
        },
        "fields" => rsx! {
            span { class: "aio-model-table__summary",
                "{model_validation_fields(&validation.rule, &fields)}"
            }
        },
        "message" => rsx! { strong { "{validation.message}" } },
        "actions" => {
            let validation_id = validation.id;
            let label = validation.message.clone();
            rsx! {
                div { class: "aio-model-table__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "编辑校验",
                        aria_label: "编辑模型校验",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editor.set(Some(ModelEditorTarget::EditValidation(validation_id)));
                        },
                        icons::Pencil { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "删除校验",
                        aria_label: "删除模型校验",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            deleting.set(Some(DefinitionDeleteTarget {
                                id: validation_id,
                                kind: "校验规则",
                                label: label.clone(),
                            }));
                        },
                        icons::Trash2 { class: "size-4" }
                    }
                }
            }
        }
        _ => rsx! { "—" },
    }
}

pub(super) fn model_validation_fields(
    rule: &crate::ModelValidationRule,
    fields: &[FieldDefinition],
) -> String {
    let field_ids = match rule {
        crate::ModelValidationRule::FieldsRequiredTogether { field_ids }
        | crate::ModelValidationRule::AtLeastOneRequired { field_ids } => field_ids.clone(),
        crate::ModelValidationRule::RequiredWhenPresent {
            field_id,
            when_field_id,
        } => vec![*field_id, *when_field_id],
    };
    field_ids
        .iter()
        .map(|field_id| {
            fields
                .iter()
                .find(|field| field.id == *field_id)
                .map(|field| field.title.clone())
                .unwrap_or_else(|| "未知字段".to_owned())
        })
        .collect::<Vec<_>>()
        .join(" + ")
}
