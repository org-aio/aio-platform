use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum ModelFieldRow {
    PrimaryKey {
        generation: crate::PrimaryKeyGeneration,
    },
    Field(FieldDefinition),
    Audit {
        kind: crate::AuditFieldKind,
        field: Option<FieldDefinition>,
        enabled: bool,
    },
}

pub(super) fn model_field_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::group(
            "identity",
            "字段",
            vec![
                DataTableColumn::leaf("title", "显示标题")
                    .width(168)
                    .fixed(DataTableFixed::Left),
                DataTableColumn::leaf("name", "字段标识").width(168),
            ],
        ),
        DataTableColumn::group(
            "schema",
            "数据结构",
            vec![
                DataTableColumn::leaf("type", "类型").width(112),
                DataTableColumn::leaf("required", "必填")
                    .width(72)
                    .align(DataTableAlign::Center),
                DataTableColumn::leaf("relation", "关联").width(196),
            ],
        ),
        DataTableColumn::group(
            "behavior",
            "页面与数据能力",
            vec![
                DataTableColumn::leaf("capabilities", "启用能力").width(280),
                DataTableColumn::leaf("validation", "字段校验").width(180),
            ],
        ),
        DataTableColumn::leaf("actions", "操作")
            .width(120)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}

#[component]
pub(super) fn ModelFieldsTable(
    model: ModelDefinition,
    all_models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let models_for_cells = all_models.clone();
    let rows = model_field_rows(&model);
    let model_for_cells = model.clone();
    rsx! {
        DataTable::<ModelFieldRow> {
            class: "aio-model-data-table",
            aria_label: "模型字段",
            rows,
            columns: model_field_columns(),
            max_height: "100%",
            empty_text: "暂无字段，请使用右上角新建字段".to_owned(),
            row_key: model_field_row_key,
            row_tone: |row: ModelFieldRow| if matches!(
                row,
                ModelFieldRow::PrimaryKey { .. } | ModelFieldRow::Audit { .. }
            ) {
                DataTableRowTone::Muted
            } else {
                DataTableRowTone::Default
            },
            render_cell: move |cell: DataTableCellContext<ModelFieldRow>| {
                model_field_cell(
                    cell,
                    model_for_cells.clone(),
                    models_for_cells.clone(),
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    generation,
                    status,
                    editor,
                    deleting,
                )
            },
        }
    }
}

pub(super) fn model_field_cell(
    cell: DataTableCellContext<ModelFieldRow>,
    model: ModelDefinition,
    all_models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    if matches!(&cell.row, ModelFieldRow::PrimaryKey { .. }) {
        return model_primary_key_cell(
            cell.row,
            &cell.column.key,
            model,
            api_base_url,
            program_id,
            version,
            generation,
            status,
        );
    }
    let ModelFieldRow::Field(field) = cell.row else {
        return model_audit_field_cell(
            cell.row,
            &cell.column.key,
            model,
            api_base_url,
            program_id,
            version,
            generation,
            status,
        );
    };
    match cell.column.key.as_str() {
        "title" => rsx! { strong { "{field.title}" } },
        "name" => rsx! { code { class: "aio-model-table__code", "{field.name}" } },
        "type" => rsx! {
            Badge { variant: BadgeVariant::Outline, "{value_type_label(&field.value_type)}" }
        },
        "required" => rsx! {
            if field.required {
                Badge { "是" }
            } else {
                span { class: "aio-model-table__muted", "否" }
            }
        },
        "relation" => model_field_relation_cell(&field, &all_models),
        "capabilities" => rsx! {
            span { class: "aio-model-table__summary", "{field_capability_summary(&field.options)}" }
        },
        "validation" => rsx! {
            span { class: "aio-model-table__summary", "{field_validation_summary(&field.options.validation)}" }
        },
        "actions" => {
            let field_id = field.id;
            let field_title = field.title.clone();
            rsx! {
                div { class: "aio-model-table__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "编辑字段",
                        aria_label: "编辑字段 {field.name}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editor.set(Some(ModelEditorTarget::EditField(field_id)));
                        },
                        icons::Pencil { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "配置字段关系",
                        aria_label: "配置字段关系 {field.name}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editor.set(Some(ModelEditorTarget::Relation(field_id)));
                        },
                        icons::Link { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "删除字段",
                        aria_label: "删除字段 {field.name}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            deleting.set(Some(DefinitionDeleteTarget {
                                id: field_id,
                                kind: "字段",
                                label: field_title.clone(),
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

pub(super) fn model_field_rows(model: &ModelDefinition) -> Vec<ModelFieldRow> {
    let audit_names = crate::AuditFieldKind::all()
        .into_iter()
        .map(crate::AuditFieldKind::default_name)
        .collect::<BTreeSet<_>>();
    let audit_field_ids = model
        .audit
        .fields
        .iter()
        .map(|field| field.field_id)
        .collect::<BTreeSet<_>>();
    let mut rows = vec![ModelFieldRow::PrimaryKey {
        generation: model.primary_key.generation,
    }];
    rows.extend(
        model
            .fields
            .iter()
            .filter(|field| {
                !audit_field_ids.contains(&field.id) && !audit_names.contains(field.name.as_str())
            })
            .cloned()
            .map(ModelFieldRow::Field)
            .collect::<Vec<_>>(),
    );
    rows.extend(crate::AuditFieldKind::all().into_iter().map(|kind| {
        let binding = model.audit.fields.iter().find(|field| field.kind == kind);
        let field = binding
            .and_then(|binding| {
                model
                    .fields
                    .iter()
                    .find(|field| field.id == binding.field_id)
            })
            .or_else(|| {
                model
                    .fields
                    .iter()
                    .find(|field| field.name == kind.default_name())
            })
            .cloned();
        ModelFieldRow::Audit {
            kind,
            field,
            enabled: binding.is_some(),
        }
    }));
    rows
}

pub(super) fn model_field_row_key(row: ModelFieldRow) -> String {
    match row {
        ModelFieldRow::PrimaryKey { .. } => "primary-key:id".to_owned(),
        ModelFieldRow::Field(field) => field.id.to_string(),
        ModelFieldRow::Audit { kind, .. } => format!("audit:{}", kind.default_name()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn model_primary_key_cell(
    row: ModelFieldRow,
    column: &str,
    model: ModelDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let ModelFieldRow::PrimaryKey {
        generation: strategy,
    } = row
    else {
        return rsx! { "—" };
    };
    match column {
        "title" => rsx! { strong { "主键" } },
        "name" => rsx! { code { class: "aio-model-table__code", "id" } },
        "type" => rsx! {
            Select {
                aria_label: "主键生成策略",
                value: strategy.as_str(),
                options: vec![
                    SelectItem::new("uuid", "UUID"),
                    SelectItem::new("auto_increment", "自增整数"),
                ],
                on_value_change: move |value: String| {
                    let generation_strategy = match value.as_str() {
                        "auto_increment" => crate::PrimaryKeyGeneration::AutoIncrement,
                        _ => crate::PrimaryKeyGeneration::Uuid,
                    };
                    let patches = vec![GraphPatch::SetProperty {
                        target_id: model.id,
                        property: crate::EditableProperty::ModelPrimaryKey,
                        value: serde_json::json!(crate::ModelPrimaryKeyDefinition {
                            generation: generation_strategy,
                        }),
                    }];
                    submit_patches(
                        api_base_url.clone(),
                        program_id.clone(),
                        version,
                        patches,
                        generation,
                        status,
                    );
                },
            }
        },
        "required" => rsx! { Badge { "是" } },
        "relation" => rsx! { span { class: "aio-model-table__muted", "系统主键" } },
        "capabilities" => rsx! { span { class: "aio-model-table__summary", "列表 · 详情" } },
        "validation" => rsx! {
            span { class: "aio-model-table__summary", "数据库生成 · 唯一" }
        },
        "actions" => rsx! { span { class: "aio-model-table__muted", "不可删除" } },
        _ => rsx! { "—" },
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn model_audit_field_cell(
    row: ModelFieldRow,
    column: &str,
    model: ModelDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let ModelFieldRow::Audit {
        kind,
        field,
        enabled,
    } = row
    else {
        return rsx! { "—" };
    };
    match column {
        "title" => rsx! { strong { "{kind.default_title()}" } },
        "name" => rsx! { code { class: "aio-model-table__code", "{kind.default_name()}" } },
        "type" => rsx! {
            Badge { variant: BadgeVariant::Outline, "{value_type_label(&kind.default_value_type())}" }
        },
        "required" => rsx! { span { class: "aio-model-table__muted", "否" } },
        "relation" => rsx! { span { class: "aio-model-table__muted", "系统维护" } },
        "capabilities" => rsx! {
            span { class: "aio-model-table__summary",
                if enabled { "已启用审计语义" } else { "未启用" }
            }
        },
        "validation" => rsx! {
            span { class: "aio-model-table__summary",
                if field.is_some() { "字段已就绪" } else { "启用时自动创建" }
            }
        },
        "actions" => rsx! {
            div { class: "aio-model-table__actions",
                Checkbox {
                    checked: Some(checkbox_state(enabled)),
                    aria_label: "启用审计字段 {kind.label()}",
                    on_checked_change: move |checked| {
                        toggle_model_audit_field(
                            model.clone(),
                            kind,
                            checkbox_is_checked(checked),
                            api_base_url.clone(),
                            program_id.clone(),
                            version,
                            generation,
                            status,
                        );
                    },
                }
            }
        },
        _ => rsx! { "—" },
    }
}

pub(super) fn model_field_relation_cell(
    field: &FieldDefinition,
    all_models: &[ModelDefinition],
) -> Element {
    let Some(relation) = &field.relation else {
        return rsx! { span { class: "aio-model-table__muted", "未配置" } };
    };
    let target_model = all_models
        .iter()
        .find(|model| model.id == relation.target_model_id);
    let model_title = target_model
        .map(|model| model.title.as_str())
        .unwrap_or("未知模型");
    let field_title = target_model
        .and_then(|model| {
            model
                .fields
                .iter()
                .find(|field| field.id == relation.target_field_id)
        })
        .map(|field| field.title.as_str())
        .unwrap_or("未知字段");
    rsx! {
        span { class: "aio-model-table__relation",
            "{relation_kind_label(relation.kind)} · {model_title}.{field_title}"
        }
    }
}

pub(super) fn field_capability_summary(options: &crate::FieldOptions) -> String {
    let mut labels = Vec::new();
    if options.list_visible {
        labels.push("列表");
    }
    if options.detail_visible {
        labels.push("详情");
    }
    if options.form_visible {
        labels.push("表单");
    }
    if options.filterable {
        labels.push("查询");
    }
    if options.sortable {
        labels.push("排序");
    }
    if options.unique {
        labels.push("唯一");
    }
    if options.ai_extract {
        labels.push("AI 提取");
    }
    if labels.is_empty() {
        "未启用".to_owned()
    } else {
        labels.join(" · ")
    }
}

pub(super) fn field_validation_summary(validation: &crate::FieldValidation) -> String {
    let mut labels = Vec::new();
    if validation.min_length.is_some() || validation.max_length.is_some() {
        labels.push("长度");
    }
    if validation.minimum.is_some() || validation.maximum.is_some() {
        labels.push("数值范围");
    }
    if validation.pattern.is_some() {
        labels.push("正则");
    }
    if validation.min_items.is_some() || validation.max_items.is_some() {
        labels.push("列表数量");
    }
    if validation.unique_items {
        labels.push("元素唯一");
    }
    if labels.is_empty() {
        "无".to_owned()
    } else {
        labels.join(" · ")
    }
}

#[component]
pub(super) fn ModelRelationsTable(
    model: ModelDefinition,
    all_models: Vec<ModelDefinition>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let models_for_cells = all_models.clone();
    let columns = vec![
        DataTableColumn::leaf("source", "本模型字段")
            .width(190)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("kind", "关联基数").width(136),
        DataTableColumn::leaf("target_model", "关联模型").width(190),
        DataTableColumn::leaf("target_field", "对端字段").width(190),
        DataTableColumn::leaf("state", "状态")
            .width(96)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("actions", "操作")
            .width(88)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ];
    rsx! {
        DataTable::<FieldDefinition> {
            class: "aio-model-data-table",
            aria_label: "模型字段关系",
            rows: model.fields.clone(),
            columns,
            max_height: "100%",
            empty_text: "请先创建字段，再为字段配置关系".to_owned(),
            row_key: |field: FieldDefinition| field.id.to_string(),
            render_cell: move |cell: DataTableCellContext<FieldDefinition>| {
                model_relation_cell(cell, models_for_cells.clone(), editor)
            },
        }
    }
}

pub(super) fn model_relation_cell(
    cell: DataTableCellContext<FieldDefinition>,
    all_models: Vec<ModelDefinition>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let field = cell.row;
    let relation = field.relation.clone();
    let target_model = relation.as_ref().and_then(|relation| {
        all_models
            .iter()
            .find(|model| model.id == relation.target_model_id)
    });
    match cell.column.key.as_str() {
        "source" => rsx! {
            div { class: "aio-model-table__identity",
                strong { "{field.title}" }
                code { "{field.name}" }
            }
        },
        "kind" => relation.as_ref().map_or_else(
            || rsx! { span { class: "aio-model-table__muted", "—" } },
            |relation| {
                rsx! {
                    Badge { variant: BadgeVariant::Outline, "{relation_kind_label(relation.kind)}" }
                }
            },
        ),
        "target_model" => target_model.map_or_else(
            || rsx! { span { class: "aio-model-table__muted", "—" } },
            |model| {
                rsx! {
                    div { class: "aio-model-table__identity",
                        strong { "{model.title}" }
                        code { "{model.name}" }
                    }
                }
            },
        ),
        "target_field" => {
            let target_field = relation.as_ref().and_then(|relation| {
                target_model.and_then(|model| {
                    model
                        .fields
                        .iter()
                        .find(|field| field.id == relation.target_field_id)
                })
            });
            target_field.map_or_else(
                || rsx! { span { class: "aio-model-table__muted", "—" } },
                |target| {
                    rsx! {
                        div { class: "aio-model-table__identity",
                            strong { "{target.title}" }
                            code { "{target.name}" }
                        }
                    }
                },
            )
        }
        "state" => rsx! {
            if relation.is_some() {
                Badge { "已连接" }
            } else {
                span { class: "aio-model-table__muted", "未配置" }
            }
        },
        "actions" => {
            let field_id = field.id;
            rsx! {
                div { class: "aio-model-table__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "配置关系",
                        aria_label: "配置关系 {field.name}",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editor.set(Some(ModelEditorTarget::Relation(field_id)));
                        },
                        if relation.is_some() {
                            icons::Pencil { class: "size-4" }
                        } else {
                            icons::Link { class: "size-4" }
                        }
                    }
                }
            }
        }
        _ => rsx! { "—" },
    }
}

pub(super) fn relation_kind_label(kind: crate::RelationKind) -> &'static str {
    match kind {
        crate::RelationKind::OneToOne => "一对一",
        crate::RelationKind::ManyToOne => "多对一",
        crate::RelationKind::OneToMany => "一对多",
        crate::RelationKind::ManyToMany => "多对多",
    }
}
