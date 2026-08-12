use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum PageSettingsTab {
    #[default]
    Layout,
    Endpoints,
}

#[component]
pub(super) fn PageRendererSettings(
    page: PageDefinition,
    program_name: String,
    models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut settings_tab: Signal<PageSettingsTab>,
    mut settings_open: Signal<bool>,
    draft: DraftSnapshot,
) -> Element {
    let page_id = page.id;
    let initial_layout = PageRendererDraft::from_definition(&page.renderer);
    let suggested_layout = suggest_user_tree_renderer(&page, &models);
    let mut layout_draft = use_signal(move || initial_layout);
    let creating_endpoint = use_signal(|| None::<PageEndpointDefinition>);
    let editing_endpoint = use_signal(|| None::<SymbolId>);
    let deleting_endpoint = use_signal(|| None::<SymbolId>);
    let expected_path = crate::convention_page_path(&program_name, &page.name);
    let layout = layout_draft();
    let selected_table = SymbolId::parse(&layout.table_model_id).ok();
    let selected_tree = SymbolId::parse(&layout.tree_model_id).ok();
    let table_model_definition =
        selected_table.and_then(|id| models.iter().find(|model| model.id == id));
    let tree_model_definition =
        selected_tree.and_then(|id| models.iter().find(|model| model.id == id));
    let table_fields = table_model_definition
        .map(|model| model.fields.as_slice())
        .unwrap_or_default();
    let tree_fields = tree_model_definition
        .map(|model| model.fields.as_slice())
        .unwrap_or_default();
    let layout_errors = layout.to_definition(&models).err().unwrap_or_default();
    let layout_valid = layout_errors.is_empty();
    let table_list_fields = table_fields
        .iter()
        .filter(|field| field.options.list_visible)
        .count();
    let table_filter_fields = table_fields
        .iter()
        .filter(|field| field.options.filterable)
        .count();
    let table_form_fields = table_fields
        .iter()
        .filter(|field| field.options.form_visible && field.options.form_editable)
        .count();
    let save_models = models.clone();
    let save_api = api_base_url.clone();
    let save_application = program_id.clone();
    let functions_api = api_base_url.clone();
    let generate_api = api_base_url;
    rsx! {
        Dialog {
            class: "aio-page-settings__panel aio-page-settings__panel--fullscreen",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    settings_open.set(false);
                }
            },
            header {
                div {
                    DialogTitle { "页面设置" }
                    DialogDescription { "{page.title} · {page.name}" }
                }
                div { class: "aio-page-settings__header-actions",
                    if let Some(message) = status() {
                        Badge { variant: BadgeVariant::Outline, "{message}" }
                    }
                    Button {
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "关闭设置",
                        aria_label: "关闭设置",
                        onclick: move |_| settings_open.set(false),
                        icons::X { class: "size-4" }
                    }
                }
            }
            nav { class: "aio-page-settings__tabs", aria_label: "页面设置视图",
                Button {
                    size: ButtonSize::Sm,
                    variant: if settings_tab() == PageSettingsTab::Layout {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    onclick: move |_| settings_tab.set(PageSettingsTab::Layout),
                    "布局"
                }
                Button {
                    size: ButtonSize::Sm,
                    variant: if settings_tab() == PageSettingsTab::Endpoints {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    onclick: move |_| settings_tab.set(PageSettingsTab::Endpoints),
                    "页面接口"
                }
            }
            if settings_tab() == PageSettingsTab::Layout {
                form { class: "aio-page-settings__form aio-page-layout-form", onsubmit: move |event| {
                    event.prevent_default();
                    let renderer = match layout_draft().to_definition(&save_models) {
                        Ok(renderer) => renderer,
                        Err(errors) => {
                            status.set(errors.first().cloned());
                            return;
                        }
                    };
                    let value = match serde_json::to_value(renderer) {
                        Ok(value) => value,
                        Err(error) => {
                            status.set(Some(format!("序列化页面设置失败: {error}")));
                            return;
                        }
                    };
                    submit_patches(
                        save_api.clone(),
                        save_application.clone(),
                        version,
                        vec![GraphPatch::SetProperty {
                            target_id: page_id,
                            property: crate::EditableProperty::PageRenderer,
                            value,
                        }],
                        generation,
                        status,
                    );
                },
                div { class: "aio-page-layout-form__content",
                    div { class: "aio-page-layout-form__workspace",
                        section { class: "aio-page-layout-form__section",
                            header {
                                h2 { "布局配置" }
                                code { "{page.name}" }
                            }
                            div { class: "aio-page-layout-form__fields",
                                label { r#for: "page-renderer-kind", "渲染方式" }
                                select {
                                    id: "page-renderer-kind",
                                    name: "renderer_kind",
                                    class: "aio-input",
                                    onchange: move |event: FormEvent| {
                                        layout_draft.with_mut(|draft| {
                                            draft.kind = PageRendererKind::from_key(&event.value());
                                        });
                                    },
                                    option {
                                        value: "convention_file",
                                        selected: layout.kind == PageRendererKind::ConventionFile,
                                        "约定文件渲染"
                                    }
                                    if layout.kind == PageRendererKind::Extension {
                                        option {
                                            value: "extension",
                                            selected: true,
                                            "扩展页面"
                                        }
                                    }
                                    option {
                                        value: "menu_tree",
                                        selected: layout.kind == PageRendererKind::MenuTree,
                                        "内置 · 程序菜单树"
                                    }
                                    option {
                                        value: "tree_table",
                                        selected: layout.kind == PageRendererKind::TreeTable,
                                        "内置 · 左树右表"
                                    }
                                    option {
                                        value: "crud_table",
                                        selected: layout.kind == PageRendererKind::CrudTable,
                                        "内置 · 增删改查表格"
                                    }
                                }
                                if let Some(suggested) = suggested_layout.clone()
                                    && layout != suggested
                                {
                                    div { class: "aio-page-layout-form__recommendation",
                                        span { "用户管理推荐：部门树 + 用户表" }
                                        Button {
                                            r#type: "button",
                                            size: ButtonSize::Sm,
                                            variant: ButtonVariant::Outline,
                                            onclick: move |_| layout_draft.set(suggested.clone()),
                                            "采用推荐"
                                        }
                                    }
                                }
                                if layout.kind == PageRendererKind::ConventionFile {
                                    div { class: "aio-page-settings__convention",
                                        code { "{expected_path}" }
                                        p { "页面模块由程序标识和页面标识确定。" }
                                        Button {
                                            r#type: "button",
                                            variant: ButtonVariant::Outline,
                                            onclick: move |_| generate_convention_file(
                                                generate_api.clone(),
                                                page_id,
                                                status,
                                            ),
                                            "生成期望文件"
                                        }
                                    }
                                } else if layout.kind != PageRendererKind::MenuTree {
                                    label { r#for: "table-model", "表格模型" }
                                    select {
                                        id: "table-model",
                                        name: "table_model_id",
                                        class: "aio-input",
                                        onchange: move |event: FormEvent| {
                                            layout_draft.with_mut(|draft| {
                                                draft.table_model_id = event.value();
                                                draft.table_relation_field_id.clear();
                                            });
                                        },
                                        option {
                                            value: "",
                                            selected: layout.table_model_id.is_empty(),
                                            "选择模型"
                                        }
                                        for model in &models {
                                            option {
                                                value: "{model.id}",
                                                selected: layout.table_model_id == model.id.to_string(),
                                                "{model.title} · {model.name}"
                                            }
                                        }
                                    }
                                    label { r#for: "page-size", "每页条数" }
                                    Input {
                                        id: "page-size",
                                        name: "page_size",
                                        class: "aio-input",
                                        r#type: "number",
                                        min: "1",
                                        max: "200",
                                        value: "{layout.page_size}",
                                        oninput: move |event: FormEvent| {
                                            layout_draft.with_mut(|draft| draft.page_size = event.value());
                                        }
                                    }
                                }
                                if layout.kind == PageRendererKind::TreeTable {
                                    label { r#for: "tree-model", "树模型" }
                                    select {
                                        id: "tree-model",
                                        name: "tree_model_id",
                                        class: "aio-input",
                                        onchange: move |event: FormEvent| {
                                            layout_draft.with_mut(|draft| {
                                                draft.tree_model_id = event.value();
                                                draft.tree_label_field_id.clear();
                                                draft.tree_parent_field_id.clear();
                                            });
                                        },
                                        option {
                                            value: "",
                                            selected: layout.tree_model_id.is_empty(),
                                            "选择树模型"
                                        }
                                        for model in &models {
                                            option {
                                                value: "{model.id}",
                                                selected: layout.tree_model_id == model.id.to_string(),
                                                "{model.title} · {model.name}"
                                            }
                                        }
                                    }
                                    label { r#for: "tree-label-field", "树标题字段" }
                                    select {
                                        id: "tree-label-field",
                                        name: "tree_label_field_id",
                                        class: "aio-input",
                                        onchange: move |event: FormEvent| {
                                            layout_draft.with_mut(|draft| draft.tree_label_field_id = event.value());
                                        },
                                        option {
                                            value: "",
                                            selected: layout.tree_label_field_id.is_empty(),
                                            "选择字段"
                                        }
                                        for field in tree_fields {
                                            option {
                                                value: "{field.id}",
                                                selected: layout.tree_label_field_id == field.id.to_string(),
                                                "{field.title} · {field.name}"
                                            }
                                        }
                                    }
                                    label { r#for: "tree-parent-field", "树父级字段" }
                                    select {
                                        id: "tree-parent-field",
                                        name: "tree_parent_field_id",
                                        class: "aio-input",
                                        onchange: move |event: FormEvent| {
                                            layout_draft.with_mut(|draft| draft.tree_parent_field_id = event.value());
                                        },
                                        option {
                                            value: "",
                                            selected: layout.tree_parent_field_id.is_empty(),
                                            "无父级字段"
                                        }
                                        for field in tree_fields {
                                            option {
                                                value: "{field.id}",
                                                selected: layout.tree_parent_field_id == field.id.to_string(),
                                                "{field.title} · {field.name}"
                                            }
                                        }
                                    }
                                    label { r#for: "table-relation-field", "表关联字段" }
                                    select {
                                        id: "table-relation-field",
                                        name: "table_relation_field_id",
                                        class: "aio-input",
                                        onchange: move |event: FormEvent| {
                                            layout_draft.with_mut(|draft| draft.table_relation_field_id = event.value());
                                        },
                                        option {
                                            value: "",
                                            selected: layout.table_relation_field_id.is_empty(),
                                            "选择字段"
                                        }
                                        for field in table_fields {
                                            option {
                                                value: "{field.id}",
                                                selected: layout.table_relation_field_id == field.id.to_string(),
                                                "{field.title} · {field.name}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        aside { class: "aio-page-layout-summary", aria_label: "布局摘要",
                            header {
                                h2 { "布局摘要" }
                                Badge {
                                    variant: if layout_valid {
                                        BadgeVariant::Secondary
                                    } else {
                                        BadgeVariant::Outline
                                    },
                                    if layout_valid { "可保存" } else { "待补充" }
                                }
                            }
                            dl {
                                div { dt { "页面类型" } dd { "{layout.kind.title()}" } }
                                if matches!(
                                    layout.kind,
                                    PageRendererKind::TreeTable | PageRendererKind::CrudTable
                                ) {
                                    div {
                                        dt { "表格模型" }
                                        dd {
                                            {table_model_definition
                                                .map(|model| model.title.as_str())
                                                .unwrap_or("未选择")}
                                        }
                                    }
                                    div { dt { "每页条数" } dd { "{layout.page_size}" } }
                                    div { dt { "列表 / 查询 / 编辑" } dd {
                                        "{table_list_fields} / {table_filter_fields} / {table_form_fields}"
                                    } }
                                }
                                if layout.kind == PageRendererKind::TreeTable {
                                    div {
                                        dt { "树模型" }
                                        dd {
                                            {tree_model_definition
                                                .map(|model| model.title.as_str())
                                                .unwrap_or("未选择")}
                                        }
                                    }
                                }
                            }
                            if let Some(model) = table_model_definition {
                                section {
                                    h3 { "表格字段" }
                                    ul {
                                        for field in model.fields.iter().filter(|field| field.options.list_visible) {
                                            li {
                                                span { "{field.title}" }
                                                code { "{field.name}" }
                                            }
                                        }
                                    }
                                }
                            }
                            if layout_errors.is_empty() {
                                p { class: "aio-page-layout-summary__ready", role: "status",
                                    "布局定义完整"
                                }
                            } else {
                                div { class: "aio-page-layout-summary__diagnostics", role: "alert",
                                    strong { "保存前检查" }
                                    ul {
                                        for error in &layout_errors {
                                            li { "{error}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                footer {
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| settings_open.set(false),
                        "取消"
                    }
                    Button { r#type: "submit", disabled: !layout_valid, "保存设置" }
                }
                }
            } else {
                div { class: "aio-page-settings__functions",
                    {endpoint_panel(
                        page.clone(),
                        &draft,
                        functions_api,
                        generation,
                        status,
                        creating_endpoint,
                        editing_endpoint,
                        deleting_endpoint,
                    )}
                }
            }
        }
    }
}

pub(super) fn generate_convention_file(
    api_base_url: String,
    page_id: SymbolId,
    mut status: Signal<Option<String>>,
) {
    spawn(async move {
        let path = format!("/api/studio/program/pages/{page_id}/convention-file");
        match post_api::<(), crate::ConventionFileResult>(&api_base_url, &path, &()).await {
            Ok(result) => status.set(Some(format!("已生成 {}", result.path))),
            Err(error) => status.set(Some(error)),
        }
    });
}

/// 用户明确采用建议时，才根据稳定模型与字段语义生成树表草稿。
pub(super) fn suggest_user_tree_renderer(
    page: &PageDefinition,
    models: &[ModelDefinition],
) -> Option<PageRendererDraft> {
    let page_text = format!("{} {}", page.name, page.title).to_lowercase();
    if !["用户", "user", "identity"]
        .iter()
        .any(|token| page_text.contains(token))
    {
        return None;
    }
    let tree = models.iter().find(|model| {
        let text = format!("{} {}", model.name, model.title).to_lowercase();
        ["部门", "组织", "department", "dept", "organization"]
            .iter()
            .any(|token| text.contains(token))
    })?;
    let table = models.iter().find(|model| {
        let text = format!("{} {}", model.name, model.title).to_lowercase();
        ["用户", "user", "account", "identity"]
            .iter()
            .any(|token| text.contains(token))
    })?;
    let label_field = tree
        .fields
        .iter()
        .find(|field| ["name", "title", "label"].contains(&field.name.as_str()))
        .or_else(|| tree.fields.first())?;
    let parent_field = tree.fields.iter().find(|field| {
        ["parent", "parent_id", "上级"]
            .iter()
            .any(|token| field.name.to_lowercase().contains(token) || field.title.contains(token))
            && field
                .relation
                .as_ref()
                .is_none_or(|relation| relation.target_model_id == tree.id)
    });
    let relation_field = table.fields.iter().find(|field| {
        field
            .relation
            .as_ref()
            .is_some_and(|relation| relation.target_model_id == tree.id)
            || ["department", "dept", "organization", "部门", "组织"]
                .iter()
                .any(|token| {
                    field.name.to_lowercase().contains(token) || field.title.contains(token)
                })
    })?;
    Some(PageRendererDraft {
        kind: PageRendererKind::TreeTable,
        extension: None,
        table_model_id: table.id.to_string(),
        page_size: "20".to_owned(),
        tree_model_id: tree.id.to_string(),
        tree_label_field_id: label_field.id.to_string(),
        tree_parent_field_id: parent_field
            .map(|field| field.id.to_string())
            .unwrap_or_default(),
        table_relation_field_id: relation_field.id.to_string(),
    })
}

pub(super) fn page_renderer_title(page: &PageDefinition) -> &'static str {
    PageRendererDraft::from_definition(&page.renderer)
        .kind
        .title()
}
