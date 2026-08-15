use super::*;

#[component]
pub(super) fn EndpointCatalogPanel(
    draft: DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut selected_page: Signal<Option<SymbolId>>,
) -> Element {
    let mut page_search = use_signal(String::new);
    let creating_endpoint = use_signal(|| None::<PageEndpointDefinition>);
    let editing_endpoint = use_signal(|| None::<SymbolId>);
    let deleting_endpoint = use_signal(|| None::<SymbolId>);
    let page_count = draft.definition.pages.len();
    let normalized_search = page_search().trim().to_lowercase();
    let mut visible_pages = draft
        .definition
        .pages
        .iter()
        .filter(|page| definition_matches_search(&page.name, &page.title, &normalized_search))
        .collect::<Vec<_>>();
    visible_pages.sort_by_key(|page| page.endpoints.is_empty());
    let current_page_id = selected_page()
        .filter(|selected_id| visible_pages.iter().any(|page| page.id == *selected_id))
        .or_else(|| {
            visible_pages
                .iter()
                .find(|page| !page.endpoints.is_empty())
                .map(|page| page.id)
        })
        .or_else(|| visible_pages.first().map(|page| page.id));
    let current_page = current_page_id.and_then(|page_id| {
        draft
            .definition
            .pages
            .iter()
            .find(|page| page.id == page_id)
            .cloned()
    });
    rsx! {
        section { class: "aio-studio-catalog",
            nav { class: "aio-studio-catalog__directory", aria_label: "接口页面目录",
                div { class: "aio-studio-catalog__directory-heading",
                    div { class: "aio-studio-catalog__directory-summary",
                        strong { "页面目录" }
                        span { "{visible_pages.len()} / {page_count}" }
                    }
                    div { class: "aio-studio-catalog__search",
                        Input {
                            class: "aio-input",
                            aria_label: "搜索接口页面",
                            placeholder: "搜索页面",
                            value: page_search(),
                            oninput: move |event: FormEvent| page_search.set(event.value()),
                        }
                        if !normalized_search.is_empty() {
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "清除页面搜索",
                                aria_label: "清除页面搜索",
                                onclick: move |_| page_search.set(String::new()),
                                icons::X { class: "size-4" }
                            }
                        }
                    }
                }
                CollectionTree::<PageDefinition> {
                    class: "aio-studio-catalog__directory-list",
                    aria_label: "接口页面目录",
                    data: CollectionTreeData::Collection(
                        visible_pages.iter().map(|page| (*page).clone()).collect()
                    ),
                    selected_key: current_page_id.map(|page_id| page_id.to_string()),
                    empty_text: "没有匹配的页面",
                    item_key: |page: PageDefinition| page.id.to_string(),
                    on_select: move |page: PageDefinition| {
                        selected_page.set(Some(page.id));
                    },
                    render_item: |item: CollectionTreeItemContext<PageDefinition>| {
                        let page = item.item;
                        rsx! {
                            div { class: "aio-studio-catalog__page-content",
                                strong { "{page.title}" }
                                code { "{page.name}" }
                                span { "{page.endpoints.len()} 个声明接口" }
                            }
                        }
                    }
                }
            }
            main { class: "aio-studio-catalog__editor",
                if let Some(page) = current_page {
                    {endpoint_panel(
                        page,
                        &draft,
                        api_base_url,
                        generation,
                        status,
                        creating_endpoint,
                        editing_endpoint,
                        deleting_endpoint,
                    )}
                } else {
                    div { class: "aio-studio-catalog__empty", "暂无页面" }
                }
            }
        }
    }
}

pub(super) fn endpoint_panel(
    page: PageDefinition,
    draft: &DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut creating_endpoint: Signal<Option<PageEndpointDefinition>>,
    mut editing_endpoint: Signal<Option<SymbolId>>,
    deleting_endpoint: Signal<Option<SymbolId>>,
) -> Element {
    let compiled_page = crate::compile_page(&draft.definition, &page);
    let compiled_endpoints = compiled_page.endpoints;
    let custom_endpoints = page.endpoints.clone();
    let page_id = page.id;
    let endpoint_count = custom_endpoints.len();
    let version = draft.version;
    let program_id = draft.program_id.clone();
    let create_endpoints = custom_endpoints.clone();
    let ai_api = api_base_url.clone();
    let page_name = page.name.clone();
    let page_title = page.title.clone();
    let endpoint_rows = compiled_endpoints
        .iter()
        .cloned()
        .map(|compiled| {
            let definition = custom_endpoints
                .iter()
                .find(|endpoint| endpoint.id.to_string() == compiled.id)
                .cloned();
            let editable = definition.is_some();
            EndpointTableRow {
                compiled,
                definition: editable.then_some(definition).flatten(),
                editable,
            }
        })
        .collect::<Vec<_>>();
    let endpoint_spans = endpoint_source_spans(&endpoint_rows);
    let endpoint_columns = endpoint_table_columns();
    let selected_row_key = editing_endpoint().map(|id| id.to_string());
    let creating_dialog_endpoint = creating_endpoint();
    let editing_dialog_endpoint = editing_endpoint().and_then(|endpoint_id| {
        custom_endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id)
            .cloned()
    });
    let deleting_dialog_endpoint = deleting_endpoint().and_then(|endpoint_id| {
        custom_endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id)
            .cloned()
    });
    let inline_api = api_base_url.clone();
    let inline_program = program_id.clone();
    rsx! {
        section { class: "aio-endpoint-workbench",
            header { class: "aio-endpoint-workbench__header",
                div {
                    h2 { "功能定义" }
                    p { "{page.title} · REST 路由与数据契约" }
                }
                Button {
                    onclick: move |_| {
                        let endpoint_id = SymbolId::new();
                        let endpoint = PageEndpointDefinition {
                            id: endpoint_id,
                            title: String::new(),
                            description: String::new(),
                            state: DefinitionState::Known,
                            method: RestMethod::Post,
                            path: next_endpoint_path(&page_name, &create_endpoints),
                            inputs: Vec::new(),
                            outputs: Vec::new(),
                        };
                        editing_endpoint.set(None);
                        creating_endpoint.set(Some(endpoint));
                    },
                    icons::Plus { class: "size-4" }
                    "新增接口"
                }
            }
            section { class: "aio-endpoint-section",
                div { class: "aio-endpoint-section__title",
                    h3 { "AI 生成" }
                }
                form { class: "aio-endpoint-ai", onsubmit: move |event| {
                    event.prevent_default();
                    let intent = form_text(&event, "endpoint_intent").trim().to_owned();
                    if intent.is_empty() {
                        let mut status = status;
                        status.set(Some("请输入接口需求".to_owned()));
                        return;
                    }
                    generate_endpoint_with_ai(
                        ai_api.clone(),
                        page_id,
                        page_title.clone(),
                        version,
                        intent,
                        generation,
                        status,
                    );
                },
                    Textarea {
                        name: "endpoint_intent",
                        class: "aio-input",
                        rows: "3",
                        placeholder: "例如：按部门批量停用用户，返回成功数量和失败用户 ID"
                    }
                    Button { r#type: "submit",
                        icons::Sparkles { class: "size-4" }
                        "生成 REST 元数据"
                    }
                }
            }
            section { class: "aio-endpoint-section",
                div { class: "aio-endpoint-section__title",
                    h3 { "接口列表" }
                    Badge { variant: BadgeVariant::Outline, "{compiled_endpoints.len()}" }
                }
                if compiled_endpoints.is_empty() {
                    {empty_panel("暂无接口定义")}
                } else {
                    DataTable::<EndpointTableRow> {
                        class: "aio-endpoint-data-table",
                        aria_label: "REST 功能定义",
                        rows: endpoint_rows,
                        columns: endpoint_columns,
                        spans: endpoint_spans,
                        selected_row_key,
                        edit_trigger: DataTableEditTrigger::Click,
                        max_height: "34rem",
                        row_key: |row: EndpointTableRow| row.compiled.id.clone(),
                        can_edit: |cell: DataTableCellContext<EndpointTableRow>| {
                            cell.row.editable
                                && matches!(cell.column.key.as_str(), "path" | "title")
                        },
                        render_cell: move |cell: DataTableCellContext<EndpointTableRow>| {
                            endpoint_table_cell(
                                cell,
                                editing_endpoint,
                                deleting_endpoint,
                            )
                        },
                        render_editor: move |edit: DataTableEditContext<EndpointTableRow>| rsx! {
                            EndpointInlineCellEditor {
                                edit,
                                api_base_url: inline_api.clone(),
                                program_id: inline_program.clone(),
                                version,
                                generation,
                                status,
                            }
                        },
                    }
                }
            }
            if let Some(endpoint) = creating_dialog_endpoint {
                EndpointEditorDialog {
                    key: "create:{endpoint.id}",
                    endpoint,
                    mode: EndpointEditorMode::Create {
                        page_id,
                        index: endpoint_count,
                    },
                    siblings: custom_endpoints.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    creating_endpoint,
                    editing_endpoint,
                }
            } else if let Some(endpoint) = editing_dialog_endpoint {
                EndpointEditorDialog {
                    key: "edit:{endpoint.id}",
                    endpoint,
                    mode: EndpointEditorMode::Edit,
                    siblings: custom_endpoints.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    creating_endpoint,
                    editing_endpoint,
                }
            }
            if let Some(endpoint) = deleting_dialog_endpoint {
                EndpointDeleteDialog {
                    endpoint,
                    api_base_url,
                    program_id,
                    version,
                    generation,
                    status,
                    editing_endpoint,
                    deleting_endpoint,
                }
            }
        }
    }
}

pub(super) fn next_endpoint_path(page_name: &str, endpoints: &[PageEndpointDefinition]) -> String {
    let mut index = endpoints.len() + 1;
    loop {
        let path = format!("/api/{page_name}/custom-endpoint-{index}");
        if endpoints.iter().all(|endpoint| endpoint.path != path) {
            return path;
        }
        index += 1;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EndpointTableRow {
    pub(super) compiled: crate::CompiledPageEndpoint,
    pub(super) definition: Option<PageEndpointDefinition>,
    pub(super) editable: bool,
}

pub(super) fn endpoint_table_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("source", "来源")
            .width(88)
            .fixed(DataTableFixed::Left),
        DataTableColumn::group(
            "request",
            "请求",
            vec![
                DataTableColumn::leaf("method", "方法")
                    .width(88)
                    .align(DataTableAlign::Center),
                DataTableColumn::leaf("path", "REST 路径")
                    .width(300)
                    .editable(),
            ],
        ),
        DataTableColumn::group(
            "description",
            "说明",
            vec![
                DataTableColumn::leaf("title", "显示名称")
                    .width(180)
                    .editable(),
                DataTableColumn::leaf("detail", "详细说明").width(260),
            ],
        ),
        DataTableColumn::group(
            "contract",
            "数据契约",
            vec![
                DataTableColumn::leaf("inputs", "入参")
                    .width(72)
                    .align(DataTableAlign::Center),
                DataTableColumn::leaf("outputs", "响应")
                    .width(72)
                    .align(DataTableAlign::Center),
            ],
        ),
        DataTableColumn::leaf("actions", "操作")
            .width(96)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}

pub(super) fn endpoint_source_spans(rows: &[EndpointTableRow]) -> Vec<DataTableSpan> {
    let mut spans = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        let source = rows[start].compiled.source;
        let mut end = start + 1;
        while end < rows.len() && rows[end].compiled.source == source {
            end += 1;
        }
        if end - start > 1 {
            spans.push(DataTableSpan::new(start, "source", end - start, 1));
        }
        start = end;
    }
    spans
}

pub(super) fn endpoint_table_cell(
    cell: DataTableCellContext<EndpointTableRow>,
    mut editing_endpoint: Signal<Option<SymbolId>>,
    mut deleting_endpoint: Signal<Option<SymbolId>>,
) -> Element {
    let editable = cell.row.editable;
    let endpoint = cell.row.compiled;
    let endpoint_id = SymbolId::parse(&endpoint.id).ok();
    match cell.column.key.as_str() {
        "source" => rsx! {
            div { class: "aio-endpoint-table__source",
                match endpoint.source {
                    PageEndpointSource::BuiltIn => rsx! {
                        Badge { variant: BadgeVariant::Outline, "内置" }
                    },
                    PageEndpointSource::Convention => rsx! {
                        Badge { variant: BadgeVariant::Outline, "约定" }
                    },
                }
            }
        },
        "method" => rsx! {
            span { class: method_class(endpoint.method), "{endpoint.method.as_str()}" }
        },
        "path" => rsx! {
            code { class: "aio-endpoint-table__path", "{endpoint.path}" }
        },
        "title" => rsx! {
            strong { "{endpoint.title}" }
        },
        "detail" => rsx! { "{endpoint.description}" },
        "inputs" => rsx! { "{endpoint.inputs.len()}" },
        "outputs" => rsx! { "{endpoint.outputs.len()}" },
        "actions" => rsx! {
            div { class: "aio-endpoint-table__actions",
                if editable && let Some(endpoint_id) = endpoint_id {
                    Button {
                        size: ButtonSize::IconSm,
                        variant: if editing_endpoint() == Some(endpoint_id) {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        },
                        title: "编辑接口",
                        aria_label: "编辑接口",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            deleting_endpoint.set(None);
                            editing_endpoint.set(Some(endpoint_id));
                        },
                        icons::Pencil { class: "size-4" }
                    }
                    Button {
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "删除接口",
                        aria_label: "删除接口",
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            editing_endpoint.set(None);
                            deleting_endpoint.set(Some(endpoint_id));
                        },
                        icons::Trash2 { class: "size-4" }
                    }
                } else if endpoint.source == PageEndpointSource::BuiltIn {
                    code { class: "aio-endpoint-table__provider", "元数据生成" }
                }
            }
        },
        _ => rsx! { "—" },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EndpointEditorMode {
    Create { page_id: SymbolId, index: usize },
    Edit,
}
