use super::*;

#[component]
pub(super) fn PagesPanel(
    draft: DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut selected_page: Signal<Option<SymbolId>>,
) -> Element {
    let mut page_search = use_signal(String::new);
    let mut creating_page = use_signal(|| false);
    let mut editing_page = use_signal(|| false);
    let mut deleting_page = use_signal(|| false);
    let mut route_editor = use_signal(|| None::<PageRouteEditorTarget>);
    let mut deleting_route = use_signal(|| None::<DefinitionDeleteTarget>);
    let mut settings_open = use_signal(|| false);
    let mut settings_tab = use_signal(PageSettingsTab::default);
    let page_count = draft.definition.pages.len();
    let normalized_search = page_search().trim().to_lowercase();
    let visible_pages = draft
        .definition
        .pages
        .iter()
        .filter(|page| definition_matches_search(&page.name, &page.title, &normalized_search))
        .collect::<Vec<_>>();
    let current_page_id = selected_page()
        .filter(|selected_id| visible_pages.iter().any(|page| page.id == *selected_id))
        .or_else(|| visible_pages.first().map(|page| page.id));
    let current_page = current_page_id.and_then(|page_id| {
        draft
            .definition
            .pages
            .iter()
            .find(|page| page.id == page_id)
            .cloned()
    });
    let current_routes = current_page_id.map_or_else(Vec::new, |page_id| {
        draft
            .definition
            .routes
            .iter()
            .filter(|route| route.page_id == page_id)
            .cloned()
            .collect::<Vec<_>>()
    });
    let menu_references = current_page_id.map_or(0, |page_id| {
        page_menu_reference_count(&draft.definition.menus, page_id)
    });
    let navigation_menu =
        current_page_id.and_then(|page_id| unique_menu_for_page(&draft.definition.menus, page_id));
    let metadata_json = current_page
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose();
    let editing_route = match route_editor() {
        Some(PageRouteEditorTarget::Edit(route_id)) => draft
            .definition
            .routes
            .iter()
            .find(|route| route.id == route_id)
            .cloned(),
        _ => None,
    };
    let route_editor_key = editing_route
        .as_ref()
        .map(|route| route.id.to_string())
        .unwrap_or_else(|| "new".to_owned());
    let page_delete_title = if menu_references > 0 {
        format!("该页面被 {menu_references} 个菜单引用，不能删除")
    } else {
        "删除页面".to_owned()
    };
    rsx! {
        section { class: "aio-studio-catalog aio-page-catalog",
            nav { class: "aio-studio-catalog__directory", aria_label: "页面目录",
                div { class: "aio-studio-catalog__directory-heading",
                    div { class: "aio-studio-catalog__directory-summary",
                        strong { "页面目录" }
                        div { class: "aio-studio-catalog__directory-actions",
                            span { "{visible_pages.len()} / {page_count}" }
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "新建页面",
                                aria_label: "新建页面",
                                onclick: move |_| creating_page.set(true),
                                icons::Plus { class: "size-4" }
                            }
                        }
                    }
                    div { class: "aio-studio-catalog__search",
                        Input {
                            class: "aio-input",
                            aria_label: "搜索页面",
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
                    aria_label: "页面目录",
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
                                span { "{page_renderer_title(&page)} · {page.endpoints.len()} 接口" }
                            }
                        }
                    }
                }
            }
            main { class: "aio-studio-catalog__editor",
                if let Some(page) = current_page.clone() {
                    section { class: "aio-page-catalog__overview",
                        header { class: "aio-page-catalog__header",
                            div {
                                h2 { "{page.title}" }
                                p { "{page.name}" }
                            }
                            div { class: "aio-page-catalog__header-actions",
                                Button {
                                    r#type: "button",
                                    size: ButtonSize::Sm,
                                    variant: ButtonVariant::Outline,
                                    onclick: move |_| editing_page.set(true),
                                    icons::Pencil { class: "size-4" }
                                    "编辑页面"
                                }
                                Button {
                                    r#type: "button",
                                    onclick: move |_| {
                                        settings_tab.set(PageSettingsTab::Layout);
                                        settings_open.set(true);
                                    },
                                    icons::Settings { class: "size-4" }
                                    "页面设置"
                                }
                                Button {
                                    r#type: "button",
                                    size: ButtonSize::IconSm,
                                    variant: ButtonVariant::Ghost,
                                    disabled: menu_references > 0,
                                    title: "{page_delete_title}",
                                    aria_label: "{page_delete_title}",
                                    onclick: move |_| deleting_page.set(true),
                                    icons::Trash2 { class: "size-4" }
                                }
                            }
                        }
                        div { class: "aio-page-catalog__content",
                            section { class: "aio-page-catalog__summary",
                                dl {
                                    div { dt { "渲染方式" } dd { "{page_renderer_title(&page)}" } }
                                    div { dt { "路由数量" } dd { "{current_routes.len()}" } }
                                    div { dt { "菜单引用" } dd { "{menu_references}" } }
                                    div { dt { "声明接口" } dd { "{page.endpoints.len()}" } }
                                }
                                section { class: "aio-page-catalog__routes",
                                    header {
                                        h3 { "路由" }
                                        div { class: "aio-page-catalog__route-toolbar",
                                            Badge { variant: BadgeVariant::Outline, "{current_routes.len()}" }
                                            Button {
                                                r#type: "button",
                                                size: ButtonSize::Sm,
                                                variant: ButtonVariant::Outline,
                                                onclick: move |_| route_editor.set(Some(PageRouteEditorTarget::Create)),
                                                icons::Plus { class: "size-4" }
                                                "新建路由"
                                            }
                                        }
                                    }
                                    if current_routes.is_empty() {
                                        p { "暂无路由" }
                                    } else {
                                        ul {
                                            for route in &current_routes {
                                                li {
                                                    div { class: "aio-page-catalog__route-identity",
                                                        code { "{route.path}" }
                                                        span { "{route.name}" }
                                                    }
                                                    div { class: "aio-page-catalog__route-actions",
                                                        if !route.required_permissions.is_empty() {
                                                            Badge {
                                                                variant: BadgeVariant::Outline,
                                                                "{route.required_permissions.len()} 权限"
                                                            }
                                                        }
                                                        Button {
                                                            r#type: "button",
                                                            size: ButtonSize::IconSm,
                                                            variant: ButtonVariant::Ghost,
                                                            title: "编辑路由 {route.name}",
                                                            aria_label: "编辑路由 {route.name}",
                                                            onclick: {
                                                                let route_id = route.id;
                                                                move |_| route_editor.set(Some(PageRouteEditorTarget::Edit(route_id)))
                                                            },
                                                            icons::Pencil { class: "size-4" }
                                                        }
                                                        Button {
                                                            r#type: "button",
                                                            size: ButtonSize::IconSm,
                                                            variant: ButtonVariant::Ghost,
                                                            disabled: current_routes.len() == 1,
                                                            title: if current_routes.len() == 1 {
                                                                "页面至少需要保留一条路由".to_owned()
                                                            } else {
                                                                format!("删除路由 {}", route.name)
                                                            },
                                                            aria_label: if current_routes.len() == 1 {
                                                                "页面至少需要保留一条路由".to_owned()
                                                            } else {
                                                                format!("删除路由 {}", route.name)
                                                            },
                                                            onclick: {
                                                                let route_id = route.id;
                                                                let route_path = route.path.clone();
                                                                move |_| deleting_route.set(Some(DefinitionDeleteTarget {
                                                                    id: route_id,
                                                                    kind: "路由",
                                                                    label: route_path.clone(),
                                                                }))
                                                            },
                                                            icons::Trash2 { class: "size-4" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            aside { class: "aio-page-catalog__metadata",
                                header {
                                    div {
                                        strong { "PageDefinition" }
                                        code { "{page.id}" }
                                    }
                                    if let Ok(Some(json)) = &metadata_json {
                                        Button {
                                            r#type: "button",
                                            size: ButtonSize::Sm,
                                            variant: ButtonVariant::Outline,
                                            title: "复制页面元数据",
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
                                        div { class: "aio-page-catalog__metadata-state",
                                            "元数据序列化失败: {error}"
                                        }
                                    },
                                    Ok(None) => rsx! {
                                        div { class: "aio-page-catalog__metadata-state", "暂无元数据" }
                                    },
                                }
                            }
                        }
                    }
                    if settings_open() {
                        PageRendererSettings {
                            key: "catalog:{page.id}:{draft.version}",
                            page: page.clone(),
                            models: draft.definition.models.clone(),
                            api_base_url: api_base_url.clone(),
                            program_id: draft.program_id.clone(),
                            version: draft.version,
                            generation,
                            status,
                            settings_tab,
                            settings_open,
                            navigation_menu: navigation_menu.clone(),
                            draft: draft.clone(),
                        }
                    }
                } else {
                    div { class: "aio-studio-catalog__empty", "暂无页面" }
                }
            }
            if creating_page() {
                PageDefinitionDialog {
                    page: None,
                    pages: draft.definition.pages.clone(),
                    models: draft.definition.models.clone(),
                    routes: draft.definition.routes.clone(),
                    root_id: draft.definition.id,
                    page_count,
                    route_count: draft.definition.routes.len(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| creating_page.set(false),
                    on_saved: move |page_id| {
                        page_search.set(String::new());
                        selected_page.set(Some(page_id));
                        creating_page.set(false);
                    },
                }
            }
            if editing_page()
                && let Some(page) = current_page.clone()
            {
                PageDefinitionDialog {
                    page: Some(page),
                    pages: draft.definition.pages.clone(),
                    models: draft.definition.models.clone(),
                    routes: draft.definition.routes.clone(),
                    root_id: draft.definition.id,
                    page_count,
                    route_count: draft.definition.routes.len(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| editing_page.set(false),
                    on_saved: move |page_id| {
                        selected_page.set(Some(page_id));
                        editing_page.set(false);
                    },
                }
            }
            if deleting_page()
                && let Some(page) = current_page.clone()
            {
                PageDeleteDialog {
                    page,
                    routes: draft.definition.routes.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    deleting: deleting_page,
                    on_deleted: move |_| selected_page.set(None),
                }
            }
            if let Some(target) = route_editor()
                && let Some(page) = current_page.clone()
            {
                PageRouteDialog {
                    key: "route:{page.id}:{route_editor_key}",
                    route: if target == PageRouteEditorTarget::Create {
                        None
                    } else {
                        editing_route.clone()
                    },
                    page,
                    routes: draft.definition.routes.clone(),
                    permissions: draft.definition.permissions.clone(),
                    root_id: draft.definition.id,
                    route_count: draft.definition.routes.len(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| route_editor.set(None),
                    on_saved: move |_| route_editor.set(None),
                }
            }
            if let Some(target) = deleting_route() {
                DefinitionDeleteDialog {
                    target,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    deleting: deleting_route,
                    on_deleted: move |_| {},
                }
            }
        }
    }
}
