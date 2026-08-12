use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageRouteEditorTarget {
    Create,
    Edit(SymbolId),
}

#[component]
pub(super) fn PageDefinitionDialog(
    page: Option<PageDefinition>,
    pages: Vec<PageDefinition>,
    models: Vec<ModelDefinition>,
    routes: Vec<RouteDefinition>,
    root_id: SymbolId,
    page_count: usize,
    route_count: usize,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<SymbolId>,
) -> Element {
    let editing = page.is_some();
    let page_id = page.as_ref().map_or_else(SymbolId::new, |page| page.id);
    let stable_name = page
        .as_ref()
        .map(|page| page.name.clone())
        .unwrap_or_default();
    let initial_title = page
        .as_ref()
        .map(|page| page.title.clone())
        .unwrap_or_default();
    let default_model_id = models
        .iter()
        .find(|model| !model.fields.is_empty())
        .map(|model| model.id.to_string())
        .unwrap_or_default();
    let default_renderer_kind = if default_model_id.is_empty() {
        PageRendererKind::ConventionFile
    } else {
        PageRendererKind::CrudTable
    };
    let mut title = use_signal(move || initial_title);
    let mut route_path = use_signal(String::new);
    let mut renderer_kind = use_signal(move || default_renderer_kind);
    let mut table_model_id = use_signal(move || default_model_id);
    let mut page_size = use_signal(|| "20".to_owned());
    let existing_pages = pages;
    let existing_routes = routes;
    let save_models = models.clone();
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-page-definition-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑页面" } else { "新建页面" } }
                    DialogDescription {
                        if editing { "页面标识创建后保持稳定，仅修改显示标题" } else { "创建可路由访问的页面定义" }
                    }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭页面编辑",
                    aria_label: "关闭页面编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_title = title().trim().to_owned();
                if next_title.is_empty() {
                    status.set(Some("页面标题不能为空".to_owned()));
                    return;
                }
                let next_name = if editing {
                    stable_name.clone()
                } else {
                    identifier_from_title(&next_title)
                };
                if next_name.is_empty() {
                    status.set(Some("页面标题无法生成有效标识，请包含中文、字母或数字".to_owned()));
                    return;
                }
                if existing_pages
                    .iter()
                    .any(|item| item.id != page_id && item.name == next_name)
                {
                    status.set(Some(format!("页面标识已存在: {next_name}")));
                    return;
                }
                let patches = if editing {
                    vec![GraphPatch::Rename {
                        target_id: page_id,
                        name: next_name,
                        title: Some(next_title),
                    }]
                } else {
                    let path = route_path().trim().to_owned();
                    if let Err(error) = validate_route_path(&path) {
                        status.set(Some(error.to_string()));
                        return;
                    }
                    if existing_routes.iter().any(|route| route.path == path) {
                        status.set(Some(format!("路由路径已存在: {path}")));
                        return;
                    }
                    let renderer = match renderer_kind() {
                        PageRendererKind::ConventionFile => {
                            crate::PageRendererDefinition::ConventionFile
                        }
                        PageRendererKind::Extension => {
                            status.set(Some("扩展页面必须从 Admin Workbench 新建".to_owned()));
                            return;
                        }
                        PageRendererKind::MenuTree => crate::PageRendererDefinition::MenuTree,
                        PageRendererKind::TreeTable | PageRendererKind::CrudTable => {
                            let draft = PageRendererDraft {
                                kind: PageRendererKind::CrudTable,
                                extension: None,
                                table_model_id: table_model_id(),
                                page_size: page_size(),
                                tree_model_id: String::new(),
                                tree_label_field_id: String::new(),
                                tree_parent_field_id: String::new(),
                                table_relation_field_id: String::new(),
                            };
                            match draft.to_definition(&save_models) {
                                Ok(renderer) => renderer,
                                Err(errors) => {
                                    status.set(errors.first().cloned());
                                    return;
                                }
                            }
                        }
                    };
                    let page = PageDefinition {
                        id: page_id,
                        name: next_name.clone(),
                        title: next_title,
                        state: DefinitionState::Known,
                        renderer,
                        endpoints: Vec::new(),
                    };
                    let route = RouteDefinition {
                        id: SymbolId::new(),
                        name: next_name,
                        path,
                        page_id,
                        state: DefinitionState::Known,
                        required_permissions: Vec::new(),
                    };
                    vec![
                        GraphPatch::Insert {
                            parent_id: root_id,
                            collection: ChildCollection::Pages,
                            index: page_count,
                            entity: GraphEntity::Page(page),
                        },
                        GraphPatch::Insert {
                            parent_id: root_id,
                            collection: ChildCollection::Routes,
                            index: route_count,
                            entity: GraphEntity::Route(route),
                        },
                    ]
                };
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    patches,
                    generation,
                    status,
                );
                on_saved.call(page_id);
            },
                div { class: "aio-definition-dialog__grid",
                    label {
                        span { "显示标题" }
                        Input {
                            class: "aio-input",
                            aria_label: "页面显示标题",
                            placeholder: "例如 工单管理",
                            value: title(),
                            oninput: move |event: FormEvent| title.set(event.value()),
                        }
                    }
                }
                if !editing {
                    section { class: "aio-definition-dialog__section",
                        h3 { "初始路由" }
                        label {
                            span { "访问路径" }
                            Input {
                                class: "aio-input",
                                aria_label: "页面初始路由",
                                placeholder: "/work-orders",
                                value: route_path(),
                                oninput: move |event: FormEvent| route_path.set(event.value()),
                            }
                        }
                    }
                    section { class: "aio-definition-dialog__section",
                        h3 { "初始布局" }
                        div { class: "aio-definition-dialog__grid",
                            label {
                                span { "页面类型" }
                                select {
                                    class: "aio-input",
                                    aria_label: "页面初始类型",
                                    onchange: move |event: FormEvent| {
                                        renderer_kind.set(PageRendererKind::from_key(&event.value()));
                                    },
                                    option {
                                        value: "convention_file",
                                        selected: renderer_kind() == PageRendererKind::ConventionFile,
                                        "约定文件"
                                    }
                                    option {
                                        value: "menu_tree",
                                        selected: renderer_kind() == PageRendererKind::MenuTree,
                                        "程序菜单树"
                                    }
                                    if !models.is_empty() {
                                        option {
                                            value: "crud_table",
                                            selected: renderer_kind() == PageRendererKind::CrudTable,
                                            "增删改查表格"
                                        }
                                    }
                                }
                            }
                            if renderer_kind() == PageRendererKind::CrudTable {
                                label {
                                    span { "数据模型" }
                                    select {
                                        class: "aio-input",
                                        aria_label: "页面初始模型",
                                        onchange: move |event: FormEvent| table_model_id.set(event.value()),
                                        for model in &models {
                                            option {
                                                value: "{model.id}",
                                                selected: table_model_id() == model.id.to_string(),
                                                "{model.title} · {model.name}"
                                            }
                                        }
                                    }
                                }
                                label {
                                    span { "每页条数" }
                                    Input {
                                        class: "aio-input",
                                        aria_label: "页面初始每页条数",
                                        r#type: "number",
                                        min: "1",
                                        max: "200",
                                        value: page_size(),
                                        oninput: move |event: FormEvent| page_size.set(event.value()),
                                    }
                                }
                            }
                        }
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
                        if editing { "保存页面" } else { "创建页面" }
                    }
                }
            }
        }
    }
}

pub(super) fn route_permission_input_name(permission_id: SymbolId) -> String {
    format!("route_permission_{permission_id}")
}

pub(super) fn route_permissions_from_form(
    event: &FormEvent,
    permissions: &[PermissionDefinition],
) -> Vec<SymbolId> {
    permissions
        .iter()
        .filter(|permission| {
            !form_text(event, &route_permission_input_name(permission.id)).is_empty()
        })
        .map(|permission| permission.id)
        .collect()
}

#[component]
pub(super) fn PageRouteDialog(
    route: Option<RouteDefinition>,
    page: PageDefinition,
    routes: Vec<RouteDefinition>,
    permissions: Vec<PermissionDefinition>,
    root_id: SymbolId,
    route_count: usize,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let editing = route.is_some();
    let route_id = route.as_ref().map_or_else(SymbolId::new, |route| route.id);
    let stable_name = route
        .as_ref()
        .map(|route| route.name.clone())
        .unwrap_or_default();
    let initial_path = route
        .as_ref()
        .map(|route| route.path.clone())
        .unwrap_or_default();
    let selected_permissions = route
        .as_ref()
        .map(|route| route.required_permissions.clone())
        .unwrap_or_default();
    let mut path = use_signal(move || initial_path);
    let existing_routes = routes;
    let save_permissions = permissions.clone();
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-page-route-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑路由" } else { "新建路由" } }
                    DialogDescription { "{page.title} · {page.name}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭路由编辑",
                    aria_label: "关闭路由编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_path = path().trim().to_owned();
                let next_name = if editing {
                    stable_name.clone()
                } else {
                    identifier_from_title(&next_path)
                };
                if next_name.is_empty() {
                    status.set(Some("路由路径无法生成有效标识".to_owned()));
                    return;
                }
                if let Err(error) = validate_route_path(&next_path) {
                    status.set(Some(error.to_string()));
                    return;
                }
                if existing_routes
                    .iter()
                    .any(|item| item.id != route_id && item.path == next_path)
                {
                    status.set(Some(format!("路由路径已存在: {next_path}")));
                    return;
                }
                let required_permissions = route_permissions_from_form(&event, &save_permissions);
                let patches = if editing {
                    vec![
                        GraphPatch::Rename {
                            target_id: route_id,
                            name: next_name,
                            title: None,
                        },
                        GraphPatch::SetProperty {
                            target_id: route_id,
                            property: crate::EditableProperty::RoutePath,
                            value: serde_json::Value::String(next_path),
                        },
                        GraphPatch::SetProperty {
                            target_id: route_id,
                            property: crate::EditableProperty::RoutePermissions,
                            value: serde_json::json!(required_permissions),
                        },
                    ]
                } else {
                    let route = RouteDefinition {
                        id: route_id,
                        name: next_name,
                        path: next_path,
                        page_id: page.id,
                        state: DefinitionState::Known,
                        required_permissions,
                    };
                    vec![GraphPatch::Insert {
                        parent_id: root_id,
                        collection: ChildCollection::Routes,
                        index: route_count,
                        entity: GraphEntity::Route(route),
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
                on_saved.call(());
            },
                div { class: "aio-definition-dialog__grid",
                    label {
                        span { "访问路径" }
                        Input {
                            class: "aio-input",
                            aria_label: "路由访问路径",
                            placeholder: "/work-orders",
                            value: path(),
                            oninput: move |event: FormEvent| path.set(event.value()),
                        }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "访问权限" }
                    if permissions.is_empty() {
                        p { class: "aio-definition-dialog__empty-state", "暂无权限定义" }
                    } else {
                        div { class: "aio-definition-dialog__choice-list",
                            for permission in &permissions {
                                label {
                                    Checkbox {
                                        name: "{route_permission_input_name(permission.id)}",
                                        default_checked: checkbox_state(selected_permissions.contains(&permission.id)),
                                        aria_label: "路由需要权限 {permission.title}",
                                    }
                                    span {
                                        strong { "{permission.title}" }
                                        code { "{permission.name}" }
                                    }
                                }
                            }
                        }
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
                        if editing { "保存路由" } else { "创建路由" }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn PageDeleteDialog(
    page: PageDefinition,
    routes: Vec<RouteDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut deleting: Signal<bool>,
    on_deleted: EventHandler<()>,
) -> Element {
    let page_id = page.id;
    let page_routes = routes
        .iter()
        .filter(|route| route.page_id == page_id)
        .count();
    let patches = delete_page_patches(&routes, page_id);
    rsx! {
        Dialog {
            class: "aio-definition-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting.set(false);
                }
            },
            DialogTitle { "删除页面" }
            DialogDescription {
                "确认删除“{page.title}”？同时删除 {page_routes} 条路由、{page.endpoints.len()} 个约定接口及对应后端文件。"
            }
            footer { class: "aio-definition-dialog__actions",
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting.set(false),
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
                            patches.clone(),
                            generation,
                            status,
                        );
                        deleting.set(false);
                        on_deleted.call(());
                    },
                    icons::Trash2 { class: "size-4" }
                    "删除页面"
                }
            }
        }
    }
}
