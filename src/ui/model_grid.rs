use super::*;

#[component]
pub(super) fn ModelGrid(
    model: ModelDefinition,
    usage: ModelUsageSummary,
    all_models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut deleting_model: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let model_id = model.id;
    let relation_count = model
        .fields
        .iter()
        .filter(|field| field.relation.is_some())
        .count();
    let usage_description = usage.description();
    let delete_disabled = usage.total() > 0;
    let delete_title = if delete_disabled {
        format!("该模型被{usage_description}引用，不能删除")
    } else {
        format!("删除模型 {}", model.title)
    };
    let mut active_tab = use_signal(ModelDesignerTab::default);
    let mut editor = use_signal(|| None::<ModelEditorTarget>);
    let deleting_definition = use_signal(|| None::<DefinitionDeleteTarget>);
    let current_editor = editor();
    let editing_field = match current_editor {
        Some(ModelEditorTarget::EditField(field_id)) => model
            .fields
            .iter()
            .find(|field| field.id == field_id)
            .cloned(),
        _ => None,
    };
    let editing_index = match current_editor {
        Some(ModelEditorTarget::EditIndex(index_id)) => model
            .indexes
            .iter()
            .find(|index| index.id == index_id)
            .cloned(),
        _ => None,
    };
    let editing_query = match current_editor {
        Some(ModelEditorTarget::EditQuery(query_id)) => model
            .queries
            .iter()
            .find(|query| query.id == query_id)
            .cloned(),
        _ => None,
    };
    let editing_validation = match current_editor {
        Some(ModelEditorTarget::EditValidation(validation_id)) => model
            .validations
            .iter()
            .find(|validation| validation.id == validation_id)
            .cloned(),
        _ => None,
    };
    rsx! {
        section { class: "aio-model-grid",
            header { class: "aio-model-grid__header",
                div { class: "aio-model-grid__heading",
                    h3 { "{model.title}" }
                    code { "{model.name}" }
                }
                div { class: "aio-model-grid__metrics",
                    span { strong { "{model.fields.len()}" } "字段" }
                    span { strong { "{relation_count}" } "关系" }
                    span { strong { "{model.queries.len()}" } "查询" }
                    span {
                        class: if delete_disabled { "is-referenced" } else { "" },
                        title: if delete_disabled { usage_description.clone() } else { "未被外部定义引用".to_owned() },
                        strong { "{usage.total()}" }
                        "引用"
                    }
                }
                div { class: "aio-model-grid__header-actions",
                    if active_tab() == ModelDesignerTab::Overview {
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            variant: ButtonVariant::Outline,
                            onclick: move |_| editor.set(Some(ModelEditorTarget::Model)),
                            icons::Pencil { class: "size-4" }
                            "编辑模型"
                        }
                        Button {
                            r#type: "button",
                            size: ButtonSize::IconSm,
                            variant: ButtonVariant::Ghost,
                            disabled: delete_disabled,
                            title: delete_title.clone(),
                            aria_label: delete_title.clone(),
                            onclick: {
                                let model_title = model.title.clone();
                                move |_| deleting_model.set(Some(DefinitionDeleteTarget {
                                    id: model_id,
                                    kind: "模型",
                                    label: model_title.clone(),
                                }))
                            },
                            icons::Trash2 { class: "size-4" }
                        }
                    } else if active_tab() == ModelDesignerTab::Fields {
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            onclick: move |_| editor.set(Some(ModelEditorTarget::CreateField)),
                            icons::Plus { class: "size-4" }
                            "新建字段"
                        }
                    } else if active_tab() == ModelDesignerTab::Indexes {
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            disabled: model.fields.is_empty(),
                            onclick: move |_| editor.set(Some(ModelEditorTarget::CreateIndex)),
                            icons::Plus { class: "size-4" }
                            "新建索引"
                        }
                    } else if active_tab() == ModelDesignerTab::Queries {
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            disabled: model.fields.is_empty(),
                            onclick: move |_| editor.set(Some(ModelEditorTarget::CreateQuery)),
                            icons::Plus { class: "size-4" }
                            "新建查询"
                        }
                    } else if active_tab() == ModelDesignerTab::Validations {
                        Button {
                            r#type: "button",
                            size: ButtonSize::Sm,
                            disabled: model.fields.len() < 2,
                            onclick: move |_| editor.set(Some(ModelEditorTarget::CreateValidation)),
                            icons::Plus { class: "size-4" }
                            "新建校验"
                        }
                    }
                }
            }
            nav { class: "aio-model-grid__tabs", aria_label: "模型设计视图",
                for (tab, label, count) in model_designer_tabs(&model) {
                    Button {
                        r#type: "button",
                        class: if active_tab() == tab { "is-active" } else { "" },
                        onclick: move |_| active_tab.set(tab),
                        span { "{label}" }
                        if let Some(count) = count {
                            Badge { variant: BadgeVariant::Outline, "{count}" }
                        }
                    }
                }
            }
            div { class: "aio-model-grid__content",
                match active_tab() {
                    ModelDesignerTab::Overview => rsx! {
                        ModelOverview { model: model.clone(), editor }
                    },
                    ModelDesignerTab::Fields => rsx! {
                        ModelFieldsTable {
                            model: model.clone(),
                            all_models: all_models.clone(),
                            editor,
                            deleting: deleting_definition,
                        }
                    },
                    ModelDesignerTab::Relations => rsx! {
                        ModelRelationsTable {
                            model: model.clone(),
                            all_models: all_models.clone(),
                            editor,
                        }
                    },
                    ModelDesignerTab::Indexes => rsx! {
                        ModelIndexesTable {
                            model: model.clone(),
                            editor,
                            deleting: deleting_definition,
                        }
                    },
                    ModelDesignerTab::Queries => rsx! {
                        ModelQueriesTable {
                            model: model.clone(),
                            all_models: all_models.clone(),
                            editor,
                            deleting: deleting_definition,
                        }
                    },
                    ModelDesignerTab::Validations => rsx! {
                        ModelValidationsTable {
                            model: model.clone(),
                            editor,
                            deleting: deleting_definition,
                        }
                    },
                }
            }
            if current_editor == Some(ModelEditorTarget::Model) {
                ModelEditorDialog {
                    model: Some(model.clone()),
                    root_id: model_id,
                    model_count: 0,
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    on_close: move |_| editor.set(None),
                    on_saved: move |_| editor.set(None),
                }
            }
            if current_editor == Some(ModelEditorTarget::Audit) {
                ModelAuditDialog {
                    model: model.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    editor,
                }
            }
            if current_editor == Some(ModelEditorTarget::CreateField) || editing_field.is_some() {
                FieldEditorDialog {
                    model_id,
                    field_count: model.fields.len(),
                    field: editing_field,
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    editor,
                }
            }
            if let Some(ModelEditorTarget::Relation(field_id)) = current_editor {
                if let Some(field) = model.fields.iter().find(|field| field.id == field_id).cloned() {
                    RelationEditorDialog {
                        field,
                        source_model: model.clone(),
                        all_models: all_models.clone(),
                        api_base_url: api_base_url.clone(),
                        program_id: program_id.clone(),
                        version,
                        generation,
                        status,
                        editor,
                    }
                }
            }
            if current_editor == Some(ModelEditorTarget::CreateIndex) || editing_index.is_some() {
                IndexEditorDialog {
                    model_id,
                    index_count: model.indexes.len(),
                    fields: model.fields.clone(),
                    index: editing_index,
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    editor,
                }
            }
            if current_editor == Some(ModelEditorTarget::CreateQuery) || editing_query.is_some() {
                QueryEditorDialog {
                    model_id,
                    query_count: model.queries.len(),
                    fields: model.fields.clone(),
                    all_models: all_models.clone(),
                    query: editing_query,
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    editor,
                }
            }
            if current_editor == Some(ModelEditorTarget::CreateValidation) || editing_validation.is_some() {
                ValidationEditorDialog {
                    model_id,
                    validation_count: model.validations.len(),
                    fields: model.fields.clone(),
                    validation: editing_validation,
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    editor,
                }
            }
            if let Some(target) = deleting_definition() {
                DefinitionDeleteDialog {
                    target,
                    api_base_url,
                    program_id,
                    version,
                    generation,
                    status,
                    deleting: deleting_definition,
                    on_deleted: move |_| {},
                }
            }
        }
    }
}

pub(super) fn model_designer_tabs(
    model: &ModelDefinition,
) -> [(ModelDesignerTab, &'static str, Option<usize>); 6] {
    let relation_count = model
        .fields
        .iter()
        .filter(|field| field.relation.is_some())
        .count();
    [
        (ModelDesignerTab::Overview, "概览", None),
        (ModelDesignerTab::Fields, "字段", Some(model.fields.len())),
        (ModelDesignerTab::Relations, "关系", Some(relation_count)),
        (ModelDesignerTab::Indexes, "索引", Some(model.indexes.len())),
        (ModelDesignerTab::Queries, "查询", Some(model.queries.len())),
        (
            ModelDesignerTab::Validations,
            "校验",
            Some(model.validations.len()),
        ),
    ]
}

#[component]
pub(super) fn ModelOverview(
    model: ModelDefinition,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    let relation_count = model
        .fields
        .iter()
        .filter(|field| field.relation.is_some())
        .count();
    rsx! {
        div { class: "aio-model-overview",
            section { class: "aio-model-overview__section",
                header {
                    div {
                        h4 { "模型信息" }
                        p { "持久化身份与结构统计" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| editor.set(Some(ModelEditorTarget::Model)),
                        icons::Pencil { class: "size-4" }
                        "编辑"
                    }
                }
                dl { class: "aio-model-overview__definition",
                    div { dt { "显示标题" } dd { "{model.title}" } }
                    div { dt { "模型标识" } dd { code { "{model.name}" } } }
                    div { dt { "字段" } dd { "{model.fields.len()}" } }
                    div { dt { "关系" } dd { "{relation_count}" } }
                    div { dt { "索引" } dd { "{model.indexes.len()}" } }
                    div { dt { "命名查询" } dd { "{model.queries.len()}" } }
                }
            }
            section { class: "aio-model-overview__section",
                header {
                    div {
                        h4 { "审计能力" }
                        p { "按语义角色自动维护正式字段" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| editor.set(Some(ModelEditorTarget::Audit)),
                        icons::Pencil { class: "size-4" }
                        "配置"
                    }
                }
                div { class: "aio-model-overview__audit",
                    if model.audit.fields.is_empty() {
                        span { class: "aio-model-overview__empty", "尚未启用审计字段" }
                    } else {
                        for audit_field in &model.audit.fields {
                            Badge { variant: BadgeVariant::Outline,
                                "{audit_field.kind.label()}"
                            }
                        }
                    }
                }
            }
        }
    }
}
