use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ModelDesignerTab {
    #[default]
    Overview,
    Fields,
    Relations,
    Indexes,
    Queries,
    Validations,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelEditorTarget {
    Model,
    Audit,
    CreateField,
    EditField(SymbolId),
    Relation(SymbolId),
    CreateIndex,
    EditIndex(SymbolId),
    CreateQuery,
    EditQuery(SymbolId),
    CreateValidation,
    EditValidation(SymbolId),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DefinitionDeleteTarget {
    pub(super) id: SymbolId,
    pub(super) kind: &'static str,
    pub(super) label: String,
}

#[component]
pub(super) fn ModelsPanel(
    draft: DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut selected_model: Signal<Option<SymbolId>>,
    preferred_model_id: Option<SymbolId>,
) -> Element {
    let mut creating_model = use_signal(|| false);
    let mut model_search = use_signal(String::new);
    let deleting_model = use_signal(|| None::<DefinitionDeleteTarget>);
    let storage_id = draft.program_id.clone();
    let root_id = draft.definition.id;
    let version = draft.version;
    let count = draft.definition.models.len();
    let normalized_search = model_search().trim().to_lowercase();
    let visible_models = draft
        .definition
        .models
        .iter()
        .filter(|model| definition_matches_search(&model.name, &model.title, &normalized_search))
        .collect::<Vec<_>>();
    let current_model_id = selected_model()
        .filter(|selected_id| visible_models.iter().any(|model| model.id == *selected_id))
        .or_else(|| {
            preferred_model_id
                .filter(|preferred_id| visible_models.iter().any(|model| model.id == *preferred_id))
        })
        .or_else(|| visible_models.first().map(|model| model.id));
    let current_model = current_model_id.and_then(|selected_id| {
        draft
            .definition
            .models
            .iter()
            .find(|model| model.id == selected_id)
            .cloned()
    });
    let metadata_json = current_model
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose();
    let current_model_usage = current_model
        .as_ref()
        .map(|model| model_usage_summary(&draft.definition, model.id))
        .unwrap_or_default();
    rsx! {
        section { class: "aio-model-designer",
            header { class: "aio-model-designer__header",
                div {
                    h2 { "模型定义" }
                    p { "{count} 个模型" }
                }
                Button {
                    r#type: "button",
                    onclick: move |_| creating_model.set(true),
                    icons::Plus { class: "size-4" }
                    "新建模型"
                }
            }
            div { class: "aio-model-workspace",
                nav { class: "aio-model-workspace__directory", aria_label: "模型目录",
                    div { class: "aio-model-workspace__directory-heading",
                        div { class: "aio-model-workspace__directory-summary",
                            strong { "模型目录" }
                            span { "{visible_models.len()} / {count}" }
                        }
                        div { class: "aio-model-workspace__search",
                            Input {
                                class: "aio-input",
                                aria_label: "搜索模型",
                                placeholder: "搜索模型",
                                value: model_search(),
                                oninput: move |event: FormEvent| model_search.set(event.value()),
                            }
                            if !normalized_search.is_empty() {
                                Button {
                                    r#type: "button",
                                    size: ButtonSize::IconSm,
                                    variant: ButtonVariant::Ghost,
                                    title: "清除模型搜索",
                                    aria_label: "清除模型搜索",
                                    onclick: move |_| model_search.set(String::new()),
                                    icons::X { class: "size-4" }
                                }
                            }
                        }
                    }
                    div { class: "aio-model-workspace__directory-list",
                        if visible_models.is_empty() {
                            div { class: "aio-model-workspace__directory-empty", "没有匹配的模型" }
                        }
                        for model in visible_models {
                            Button {
                                r#type: "button",
                                class: if Some(model.id) == current_model_id {
                                    "aio-model-workspace__model aio-model-workspace__model--active"
                                } else {
                                    "aio-model-workspace__model"
                                },
                                onclick: {
                                    let model_id = model.id;
                                    move |_| selected_model.set(Some(model_id))
                                },
                                strong { "{model.title}" }
                                code { "{model.name}" }
                                span { "{model.fields.len()} 字段 · {model.indexes.len()} 索引" }
                            }
                        }
                    }
                }
                main { class: "aio-model-workspace__editor",
                    if let Some(model) = current_model.clone() {
                        ModelGrid {
                            key: "{model.id}",
                            model,
                            usage: current_model_usage,
                            all_models: draft.definition.models.clone(),
                            api_base_url: api_base_url.clone(),
                            program_id: storage_id.clone(),
                            version,
                            generation,
                            status,
                            deleting_model,
                        }
                    } else {
                        div { class: "aio-model-designer__empty", "暂无模型" }
                    }
                }
                aside { class: "aio-model-workspace__metadata",
                    header {
                        div {
                            strong { "元数据 JSON" }
                            if let Some(model) = current_model.as_ref() {
                                code { "{model.name}" }
                            }
                        }
                        if let Ok(Some(json)) = &metadata_json {
                            Button {
                                size: ButtonSize::Sm,
                                variant: ButtonVariant::Outline,
                                title: "复制元数据 JSON",
                                onclick: {
                                    let json = json.clone();
                                    move |_| copy_json_to_clipboard(json.clone(), status)
                                },
                                icons::Copy { class: "size-4" }
                                "复制"
                            }
                        }
                    }
                    match &metadata_json {
                        Ok(Some(json)) => rsx! { pre { "{json}" } },
                        Err(error) => rsx! {
                            div { class: "aio-model-workspace__metadata-error",
                                "元数据序列化失败: {error}"
                            }
                        },
                        Ok(None) => rsx! {
                            div { class: "aio-model-workspace__metadata-empty", "暂无元数据" }
                        },
                    }
                }
            }
            if creating_model() {
                ModelEditorDialog {
                    model: None,
                    root_id,
                    model_count: count,
                    api_base_url: api_base_url.clone(),
                    program_id: storage_id.clone(),
                    version,
                    generation,
                    status,
                    on_close: move |_| creating_model.set(false),
                    on_saved: move |model_id| {
                        model_search.set(String::new());
                        selected_model.set(Some(model_id));
                        creating_model.set(false);
                    },
                }
            }
            if let Some(target) = deleting_model() {
                DefinitionDeleteDialog {
                    target,
                    api_base_url,
                    program_id: storage_id,
                    version,
                    generation,
                    status,
                    deleting: deleting_model,
                    on_deleted: move |_| selected_model.set(None),
                }
            }
        }
    }
}

#[component]
pub(super) fn ModelEditorDialog(
    model: Option<ModelDefinition>,
    root_id: SymbolId,
    model_count: usize,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<SymbolId>,
) -> Element {
    let editing = model.is_some();
    let model_id = model.as_ref().map_or_else(SymbolId::new, |model| model.id);
    let initial_name = model
        .as_ref()
        .map(|model| model.name.clone())
        .unwrap_or_default();
    let initial_title = model
        .as_ref()
        .map(|model| model.title.clone())
        .unwrap_or_default();
    let mut name = use_signal(move || initial_name);
    let mut title = use_signal(move || initial_title);
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-model-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑模型" } else { "新建模型" } }
                    DialogDescription {
                        if editing { "修改稳定模型标识与显示标题" } else { "声明一个新的持久化领域模型" }
                    }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭模型编辑",
                    aria_label: "关闭模型编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form {
                class: "aio-definition-dialog__form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let next_name = name().trim().to_owned();
                    let next_title = title().trim().to_owned();
                    if next_name.is_empty() || next_title.is_empty() {
                        status.set(Some("模型标识和标题不能为空".to_owned()));
                        return;
                    }
                    let patches = if editing {
                        vec![GraphPatch::Rename {
                            target_id: model_id,
                            name: next_name,
                            title: Some(next_title),
                        }]
                    } else {
                        let definition = ModelDefinition {
                            id: model_id,
                            name: next_name,
                            title: next_title,
                            state: DefinitionState::Known,
                            fields: Vec::new(),
                            indexes: Vec::new(),
                            queries: Vec::new(),
                            validations: Vec::new(),
                            audit: crate::ModelAuditDefinition::default(),
                        };
                        vec![GraphPatch::Insert {
                            parent_id: root_id,
                            collection: ChildCollection::Models,
                            index: model_count,
                            entity: GraphEntity::Model(definition),
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
                    on_saved.call(model_id);
                },
                label {
                    span { "模型标识" }
                    Input {
                        class: "aio-input",
                        aria_label: "模型标识",
                        placeholder: "例如 work_order",
                        value: name(),
                        oninput: move |event: FormEvent| name.set(event.value()),
                    }
                }
                label {
                    span { "显示标题" }
                    Input {
                        class: "aio-input",
                        aria_label: "模型显示标题",
                        placeholder: "例如 工单",
                        value: title(),
                        oninput: move |event: FormEvent| title.set(event.value()),
                    }
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| on_close.call(()),
                        "取消"
                    }
                    Button {
                        r#type: "submit",
                        icons::Save { class: "size-4" }
                        if editing { "保存模型" } else { "创建模型" }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn DefinitionDeleteDialog(
    target: DefinitionDeleteTarget,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
    on_deleted: EventHandler<()>,
) -> Element {
    let target_id = target.id;
    rsx! {
        Dialog {
            class: "aio-definition-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting.set(None);
                }
            },
            DialogTitle { "删除{target.kind}" }
            DialogDescription { "确认删除“{target.label}”？此操作不可恢复。" }
            footer { class: "aio-definition-dialog__actions",
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting.set(None),
                    "取消"
                }
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Destructive,
                    onclick: move |_| {
                        submit_patches(
                            api_base_url.clone(),
                            program_id.clone(),
                            version,
                            vec![GraphPatch::Delete { target_id }],
                            generation,
                            status,
                        );
                        deleting.set(None);
                        on_deleted.call(());
                    },
                    icons::Trash2 { class: "size-4" }
                    "删除"
                }
            }
        }
    }
}

pub(super) fn copy_json_to_clipboard(json: String, mut status: Signal<Option<String>>) {
    #[cfg(target_arch = "wasm32")]
    spawn(async move {
        let result = crate::browser_http::write_clipboard(&json).await;
        status.set(Some(if result.is_ok() {
            "元数据 JSON 已复制".to_owned()
        } else {
            "复制失败，请检查浏览器剪贴板权限".to_owned()
        }));
    });

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = json;
        status.set(Some("剪贴板仅在 Web 界面可用".to_owned()));
    }
}
