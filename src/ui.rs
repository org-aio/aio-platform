use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use crate::{
    ChildCollection, DefinitionState, DraftSnapshot, EndpointInputDefinition,
    EndpointInputLocation, EndpointOutputDefinition, FieldDefinition, GraphEntity, GraphPatch,
    GraphPatchBatch, IndexPurpose, MenuDefinition, ModelDefinition, ModelIndexDefinition,
    PageDefinition, PageEndpointDefinition, PageEndpointSource, PatchOrigin, PermissionDefinition,
    RestMethod, RouteDefinition, SymbolId, ValueType, VibeRunAccepted, VibeRunRequest,
};
use dioxus::prelude::*;
use serde_json::Value;

use crate::browser_http::{get_api, patch_api, post_api};
use crate::design_system::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant};
use gloo_timers::future::TimeoutFuture;

#[component]
pub fn StudioPage(api_base_url: String, mut selected_scene: Signal<Option<SymbolId>>) -> Element {
    let draft_generation = use_signal(|| 0_u64);
    let editing_menu = use_signal(|| None::<SymbolId>);
    let collapsed_menus = use_signal(BTreeSet::<SymbolId>::new);
    let status = use_signal(|| None::<String>);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = draft_generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    use_effect(move || {
        if let Some(Ok(draft)) = draft.read().as_ref()
            && !draft
                .definition
                .menus
                .iter()
                .any(|scene| Some(scene.id) == selected_scene())
        {
            selected_scene.set(draft.definition.menus.first().map(|scene| scene.id));
        }
    });

    let draft_snapshot = draft.read().as_ref().cloned();

    rsx! {
        section { class: "aio-studio-shell min-h-[calc(100vh-8rem)] border bg-background",
            header { class: "flex min-h-12 items-center justify-end border-b px-3",
                if let Some(message) = status() {
                    Badge { variant: BadgeVariant::Outline, "{message}" }
                }
            }
            main { class: "min-w-0 p-4",
                match draft_snapshot {
                    Some(Ok(draft)) => scenes_panel(
                        &draft,
                        selected_scene(),
                        api_base_url.clone(),
                        draft_generation,
                        status,
                        editing_menu,
                        collapsed_menus,
                    ),
                    Some(Err(error)) => empty_panel(&error),
                    None => empty_panel("正在加载 Draft"),
                }
            }
        }
    }
}

/// 管理模式从左侧栏直接打开当前页面的设置与功能定义。
#[component]
pub(crate) fn AdminPageEditor(
    api_base_url: String,
    page_id: SymbolId,
    settings_open: Signal<bool>,
) -> Element {
    let generation = use_signal(|| 0_u64);
    let status = use_signal(|| None::<String>);
    let selected_model = use_signal(|| None::<SymbolId>);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    let Some(result) = draft.read().as_ref().cloned() else {
        return empty_panel("正在加载页面设置");
    };
    let draft = match result {
        Ok(draft) => draft,
        Err(error) => return empty_panel(&error),
    };
    let Some(page) = draft
        .definition
        .pages
        .iter()
        .find(|page| page.id == page_id)
        .cloned()
    else {
        return empty_panel("当前页面不在 Draft 中");
    };
    rsx! {
        PageRendererSettings {
            key: "admin:{page_id}:{draft.version}",
            page,
            program_name: draft.definition.name.clone(),
            models: draft.definition.models.clone(),
            api_base_url,
            program_id: draft.program_id.clone(),
            version: draft.version,
            generation,
            status,
            settings_open,
            selected_model,
            draft,
        }
    }
}

/// 管理模式从左侧栏直接向当前场景新增菜单和页面。
#[component]
pub(crate) fn AdminMenuCreator(
    api_base_url: String,
    scene_id: SymbolId,
    creator_open: Signal<bool>,
) -> Element {
    let generation = use_signal(|| 0_u64);
    let mut status = use_signal(|| None::<String>);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    let Some(result) = draft.read().as_ref().cloned() else {
        return empty_panel("正在加载 Program 定义");
    };
    let draft = match result {
        Ok(draft) => draft,
        Err(error) => return empty_panel(&error),
    };
    let page_count = draft.definition.pages.len();
    let child_count = menu_child_count(&draft.definition.menus, scene_id).unwrap_or_default();
    let program_name = draft.definition.name.clone();
    let submit_api = api_base_url;
    let submit_program = draft.program_id.clone();
    rsx! {
        div { class: "aio-page-settings__backdrop", onclick: move |_| creator_open.set(false) }
        aside { class: "aio-page-settings__panel", aria_label: "添加菜单",
            header {
                div {
                    strong { "添加菜单" }
                    p { "当前场景" }
                }
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭",
                    aria_label: "关闭",
                    onclick: move |_| creator_open.set(false),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-page-settings__form", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name").trim().to_owned();
                let title = form_text(&event, "title").trim().to_owned();
                let path = form_text(&event, "path").trim().to_owned();
                if name.is_empty() || title.is_empty() || !path.starts_with('/') {
                    status.set(Some("页面标识、标题不能为空，路由必须以 / 开头".to_owned()));
                    return;
                }
                let page_id = SymbolId::new();
                let page = PageDefinition {
                    id: page_id,
                    name: name.clone(),
                    title: title.clone(),
                    state: DefinitionState::Known,
                    renderer: crate::PageRendererDefinition::ConventionFile,
                    endpoints: Vec::new(),
                };
                let route = RouteDefinition {
                    id: SymbolId::new(),
                    name: name.clone(),
                    path,
                    page_id,
                    state: DefinitionState::Known,
                    required_permissions: Vec::new(),
                };
                let menu = MenuDefinition {
                    id: SymbolId::new(),
                    name,
                    title,
                    state: DefinitionState::Known,
                    icon: None,
                    page_id: Some(page_id),
                    enabled: true,
                    children: Vec::new(),
                    required_permissions: Vec::new(),
                    row_actions: crate::MenuRowActions::default(),
                };
                submit_patches(
                    submit_api.clone(),
                    submit_program.clone(),
                    draft.version,
                    vec![
                        GraphPatch::Insert {
                            parent_id: draft.definition.id,
                            collection: ChildCollection::Pages,
                            index: page_count,
                            entity: GraphEntity::Page(page),
                        },
                        GraphPatch::Insert {
                            parent_id: draft.definition.id,
                            collection: ChildCollection::Routes,
                            index: draft.definition.routes.len(),
                            entity: GraphEntity::Route(route),
                        },
                        GraphPatch::Insert {
                            parent_id: scene_id,
                            collection: ChildCollection::MenuChildren,
                            index: child_count,
                            entity: GraphEntity::Menu(menu),
                        },
                    ],
                    generation,
                    status,
                );
                creator_open.set(false);
            },
                label { r#for: "admin-page-name", "页面标识" }
                input {
                    id: "admin-page-name",
                    name: "name",
                    class: "aio-input",
                    placeholder: "例如 order-list"
                }
                label { r#for: "admin-page-title", "页面标题" }
                input {
                    id: "admin-page-title",
                    name: "title",
                    class: "aio-input",
                    placeholder: "例如 订单管理"
                }
                label { r#for: "admin-page-path", "路由" }
                input {
                    id: "admin-page-path",
                    name: "path",
                    class: "aio-input",
                    placeholder: "/orders"
                }
                p { class: "text-xs text-muted-foreground", "约定页面模块：{program_name} / 页面标识" }
                if let Some(message) = status() {
                    p { class: "text-xs text-destructive", "{message}" }
                }
                footer {
                    Button { button_type: "submit", "添加" }
                    Button {
                        button_type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| creator_open.set(false),
                        "取消"
                    }
                }
            }
        }
    }
}

/// 管理模式从顶部场景栏直接新增根场景。
#[component]
pub(crate) fn AdminSceneCreator(
    api_base_url: String,
    pending_scene: Signal<Option<SymbolId>>,
    creator_open: Signal<bool>,
) -> Element {
    let generation = use_signal(|| 0_u64);
    let mut status = use_signal(|| None::<String>);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    let Some(result) = draft.read().as_ref().cloned() else {
        return empty_panel("正在加载 Program 定义");
    };
    let draft = match result {
        Ok(draft) => draft,
        Err(error) => return empty_panel(&error),
    };
    let scene_count = draft.definition.menus.len();
    let submit_program = draft.program_id.clone();
    rsx! {
        div { class: "aio-page-settings__backdrop", onclick: move |_| creator_open.set(false) }
        aside { class: "aio-page-settings__panel", aria_label: "添加场景",
            header {
                strong { "添加场景" }
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭",
                    aria_label: "关闭",
                    onclick: move |_| creator_open.set(false),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-page-settings__form", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name").trim().to_owned();
                let title = form_text(&event, "title").trim().to_owned();
                if name.is_empty() || title.is_empty() {
                    status.set(Some("场景标识和标题不能为空".to_owned()));
                    return;
                }
                let scene_id = SymbolId::new();
                let scene = MenuDefinition {
                    id: scene_id,
                    name,
                    title,
                    state: DefinitionState::Known,
                    icon: None,
                    page_id: None,
                    enabled: true,
                    children: Vec::new(),
                    required_permissions: Vec::new(),
                    row_actions: crate::MenuRowActions::default(),
                };
                submit_patches(
                    api_base_url.clone(),
                    submit_program.clone(),
                    draft.version,
                    vec![GraphPatch::Insert {
                        parent_id: draft.definition.id,
                        collection: ChildCollection::Menus,
                        index: scene_count,
                        entity: GraphEntity::Menu(scene),
                    }],
                    generation,
                    status,
                );
                pending_scene.set(Some(scene_id));
                creator_open.set(false);
            },
                label { r#for: "admin-scene-name", "场景标识" }
                input {
                    id: "admin-scene-name",
                    name: "name",
                    class: "aio-input",
                    placeholder: "例如 operations"
                }
                label { r#for: "admin-scene-title", "场景标题" }
                input {
                    id: "admin-scene-title",
                    name: "title",
                    class: "aio-input",
                    placeholder: "例如 运维中心"
                }
                if let Some(message) = status() {
                    p { class: "text-xs text-destructive", "{message}" }
                }
                footer {
                    Button { button_type: "submit", "添加" }
                    Button {
                        button_type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| creator_open.set(false),
                        "取消"
                    }
                }
            }
        }
    }
}

fn menu_child_count(menus: &[MenuDefinition], target: SymbolId) -> Option<usize> {
    menus.iter().find_map(|menu| {
        if menu.id == target {
            return Some(menu.children.len());
        }
        menu_child_count(&menu.children, target)
    })
}

fn scenes_panel(
    draft: &DraftSnapshot,
    selected_scene: Option<SymbolId>,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    editing_menu: Signal<Option<SymbolId>>,
    collapsed_menus: Signal<BTreeSet<SymbolId>>,
) -> Element {
    let storage_id = draft.program_id.clone();
    let version = draft.version;
    let root_id = draft.definition.id;
    let scene_count = draft.definition.menus.len();
    let selected = selected_scene.and_then(|scene_id| {
        draft
            .definition
            .menus
            .iter()
            .cloned()
            .enumerate()
            .find(|(_, scene)| scene.id == scene_id)
    });
    let table_context = MenuTableContext {
        api_base_url,
        program_id: storage_id,
        version,
        pages: Arc::new(draft.definition.pages.clone()),
        routes: Arc::new(draft.definition.routes.clone()),
        permissions: Arc::new(draft.definition.permissions.clone()),
        generation,
        status,
        editing_menu,
        collapsed_menus,
    };
    rsx! {
        section { class: "aio-menu-management",
            header { class: "aio-menu-management__toolbar",
                div {
                    h2 { "菜单" }
                }
            }
            div { class: "aio-menu-table-scroll",
                div { class: "aio-menu-table", role: "table", aria_label: "场景与菜单",
                    div { class: "aio-menu-table__header", role: "row",
                        span { role: "columnheader", "菜单名称" }
                        span { role: "columnheader", "图标" }
                        span { role: "columnheader", "排序" }
                        span { role: "columnheader", "权限标识" }
                        span { role: "columnheader", "路由" }
                        span { role: "columnheader", "页面" }
                        span { role: "columnheader", "状态" }
                        span { role: "columnheader", "操作" }
                    }
                    if let Some((index, scene)) = selected {
                        {menu_table_rows(
                            scene,
                            0,
                            index,
                            root_id,
                            ChildCollection::Menus,
                            scene_count,
                            table_context,
                        )}
                    } else {
                        div { class: "aio-menu-table__empty", "暂无场景" }
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct MenuTableContext {
    api_base_url: String,
    program_id: String,
    version: i64,
    pages: Arc<Vec<PageDefinition>>,
    routes: Arc<Vec<RouteDefinition>>,
    permissions: Arc<Vec<PermissionDefinition>>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    editing_menu: Signal<Option<SymbolId>>,
    collapsed_menus: Signal<BTreeSet<SymbolId>>,
}

fn menu_table_rows(
    menu: MenuDefinition,
    depth: usize,
    position: usize,
    parent_id: SymbolId,
    collection: ChildCollection,
    sibling_count: usize,
    context: MenuTableContext,
) -> Element {
    let menu_id = menu.id;
    let children = menu.children.clone();
    let child_count = children.len();
    let is_collapsed = (context.collapsed_menus)().contains(&menu_id);
    let is_editing = (context.editing_menu)() == Some(menu_id);
    let page = menu
        .page_id
        .and_then(|page_id| context.pages.iter().find(|page| page.id == page_id))
        .cloned();
    let route = page.as_ref().and_then(|page| {
        context
            .routes
            .iter()
            .find(|route| route.page_id == page.id)
            .cloned()
    });
    let route_path = route
        .as_ref()
        .map(|route| route.path.as_str())
        .unwrap_or("—");
    let page_name = page.as_ref().map(|page| page.name.as_str()).unwrap_or("—");
    let permission_names = menu
        .required_permissions
        .iter()
        .filter_map(|permission_id| {
            context
                .permissions
                .iter()
                .find(|permission| permission.id == *permission_id)
                .map(|permission| permission.name.as_str())
        })
        .collect::<Vec<_>>()
        .join(", ");
    let permission_names = if permission_names.is_empty() {
        "—".to_owned()
    } else {
        permission_names
    };
    let icon = menu
        .icon
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("—");
    let row_class = if depth == 0 {
        "aio-menu-table__row aio-menu-table__row--scene"
    } else {
        "aio-menu-table__row"
    };
    let indent_style = format!("--aio-menu-depth: {depth}");
    let mut toggle_context = context.clone();
    let drop_context = context.clone();
    let mut edit_context = context.clone();
    let mut add_context = context.clone();
    let delete_context = context.clone();
    let enable_context = context.clone();
    rsx! {
        div { class: "aio-menu-table__contents",
            div {
                class: row_class,
                role: "row",
                draggable: "true",
                ondragstart: move |event| {
                    let _ = event.data_transfer().set_data("application/x-aio-menu", &menu_id.to_string());
                },
                ondragover: move |event| event.prevent_default(),
                ondrop: move |event| {
                    event.prevent_default();
                    event.stop_propagation();
                    let Some(target_id) = event.data_transfer().get_data("application/x-aio-menu") else { return; };
                    let Ok(target_id) = SymbolId::parse(&target_id) else { return; };
                    if target_id == menu_id { return; }
                    submit_patches(
                        drop_context.api_base_url.clone(),
                        drop_context.program_id.clone(),
                        drop_context.version,
                        vec![GraphPatch::Move {
                            target_id,
                            parent_id: menu_id,
                            collection: ChildCollection::MenuChildren,
                            index: child_count,
                        }],
                        drop_context.generation,
                        drop_context.status,
                    );
                },
                div { class: "aio-menu-table__name", role: "cell", style: indent_style,
                    if child_count == 0 {
                        span { class: "aio-menu-table__tree-spacer" }
                    } else {
                        button {
                            class: if is_collapsed { "aio-menu-table__tree-toggle" } else { "aio-menu-table__tree-toggle aio-menu-table__tree-toggle--open" },
                            r#type: "button",
                            title: if is_collapsed { "展开菜单" } else { "收起菜单" },
                            aria_label: if is_collapsed { "展开菜单" } else { "收起菜单" },
                            onclick: move |_| toggle_context.collapsed_menus.with_mut(|items| {
                                if !items.remove(&menu_id) {
                                    items.insert(menu_id);
                                }
                            }),
                            "›"
                        }
                    }
                    span { class: "aio-menu-table__title", "{menu.title}" }
                }
                span { class: "aio-menu-table__icon", role: "cell", title: "{icon}", "{icon}" }
                span { role: "cell", "{position + 1}" }
                code { class: "aio-menu-table__code", role: "cell", "{permission_names}" }
                code { class: "aio-menu-table__code", role: "cell", "{route_path}" }
                span { class: "aio-menu-table__page", role: "cell", "{page_name}" }
                label { class: "aio-menu-switch", title: if menu.enabled { "已启用" } else { "已停用" },
                    input {
                        r#type: "checkbox",
                        checked: menu.enabled,
                        aria_label: if menu.enabled { "停用菜单" } else { "启用菜单" },
                        onchange: move |event| submit_patches(
                            enable_context.api_base_url.clone(),
                            enable_context.program_id.clone(),
                            enable_context.version,
                            vec![GraphPatch::SetProperty {
                                target_id: menu_id,
                                property: crate::EditableProperty::MenuEnabled,
                                value: serde_json::Value::Bool(event.checked()),
                            }],
                            enable_context.generation,
                            enable_context.status,
                        ),
                    }
                    span { aria_hidden: "true" }
                }
                div { class: "aio-menu-table__row-actions", role: "cell",
                    button {
                        r#type: "button",
                        onclick: move |_| edit_context.editing_menu.set(Some(menu_id)),
                        "修改"
                    }
                    button {
                        r#type: "button",
                        onclick: move |_| {
                            add_context.collapsed_menus.with_mut(|items| {
                                items.remove(&menu_id);
                            });
                            submit_patches(
                                add_context.api_base_url.clone(),
                                add_context.program_id.clone(),
                                add_context.version,
                                vec![GraphPatch::Insert {
                                    parent_id: menu_id,
                                    collection: ChildCollection::MenuChildren,
                                    index: child_count,
                                    entity: GraphEntity::Menu(MenuDefinition {
                                        id: SymbolId::new(),
                                        name: format!("menu-{}", child_count + 1),
                                        title: "新菜单".to_owned(),
                                        state: DefinitionState::Known,
                                        icon: None,
                                        page_id: None,
                                        enabled: true,
                                        children: Vec::new(),
                                        required_permissions: Vec::new(),
                                        row_actions: crate::MenuRowActions::default(),
                                    }),
                                }],
                                add_context.generation,
                                add_context.status,
                            );
                        },
                        "新增"
                    }
                    button {
                        class: "aio-menu-table__delete",
                        r#type: "button",
                        onclick: move |_| submit_patches(
                            delete_context.api_base_url.clone(),
                            delete_context.program_id.clone(),
                            delete_context.version,
                            vec![GraphPatch::Delete { target_id: menu_id }],
                            delete_context.generation,
                            delete_context.status,
                        ),
                        "删除"
                    }
                }
            }
            if is_editing {
                {menu_edit_row(
                    menu.clone(),
                    route,
                    position,
                    parent_id,
                    collection,
                    sibling_count,
                    context.clone(),
                )}
            }
            if !is_collapsed {
                for (child_position, child) in children.into_iter().enumerate() {
                    {menu_table_rows(
                            child,
                            depth + 1,
                            child_position,
                            menu_id,
                            ChildCollection::MenuChildren,
                            child_count,
                            context.clone(),
                    )}
                }
            }
        }
    }
}

fn menu_edit_row(
    menu: MenuDefinition,
    route: Option<RouteDefinition>,
    position: usize,
    parent_id: SymbolId,
    collection: ChildCollection,
    sibling_count: usize,
    context: MenuTableContext,
) -> Element {
    let menu_id = menu.id;
    let current_page_id = menu.page_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_permission = menu.required_permissions.first().copied();
    let detail_access = menu_action_value(&menu.row_actions.detail);
    let edit_access = menu_action_value(&menu.row_actions.edit);
    let delete_access = menu_action_value(&menu.row_actions.delete);
    let route_id = route.as_ref().map(|value| value.id);
    let route_path = route
        .as_ref()
        .map(|value| value.path.clone())
        .unwrap_or_default();
    let mut submit_context = context.clone();
    let mut cancel_editing = context.editing_menu;
    rsx! {
        form { class: "aio-menu-table__editor", onsubmit: move |event| {
            event.prevent_default();
            let name = form_text(&event, "name");
            let title = form_text(&event, "title");
            if name.trim().is_empty() || title.trim().is_empty() {
                submit_context.status.set(Some("菜单名称和标题不能为空".to_owned()));
                return;
            }
            let icon = form_text(&event, "icon");
            let page_id = form_text(&event, "page_id");
            let permission_id = form_text(&event, "permission_id");
            let row_actions_value = match serde_json::to_value(crate::MenuRowActions {
                detail: menu_action_from_form(&event, "detail_access"),
                edit: menu_action_from_form(&event, "edit_access"),
                delete: menu_action_from_form(&event, "delete_access"),
            }) {
                Ok(value) => value,
                Err(error) => {
                    submit_context.status.set(Some(format!("序列化行操作权限失败: {error}")));
                    return;
                }
            };
            let mut patches = vec![
                GraphPatch::Rename {
                    target_id: menu_id,
                    name,
                    title: Some(title),
                },
                GraphPatch::SetProperty {
                    target_id: menu_id,
                    property: crate::EditableProperty::Icon,
                    value: if icon.trim().is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(icon)
                    },
                },
                GraphPatch::SetProperty {
                    target_id: menu_id,
                    property: crate::EditableProperty::MenuPage,
                    value: if page_id.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(page_id.clone())
                    },
                },
                GraphPatch::SetProperty {
                    target_id: menu_id,
                    property: crate::EditableProperty::MenuPermissions,
                    value: if permission_id.is_empty() {
                        serde_json::json!([])
                    } else {
                        serde_json::json!([permission_id])
                    },
                },
                GraphPatch::SetProperty {
                    target_id: menu_id,
                    property: crate::EditableProperty::MenuRowActions,
                    value: row_actions_value,
                },
            ];
            if page_id == current_page_id
                && let Some(route_id) = route_id
            {
                patches.push(GraphPatch::SetProperty {
                    target_id: route_id,
                    property: crate::EditableProperty::RoutePath,
                    value: serde_json::Value::String(form_text(&event, "path")),
                });
            }
            let requested_position = form_text(&event, "sort")
                .parse::<usize>()
                .unwrap_or(position + 1)
                .saturating_sub(1)
                .min(sibling_count.saturating_sub(1));
            if requested_position != position {
                patches.push(GraphPatch::Move {
                    target_id: menu_id,
                    parent_id,
                    collection,
                    index: requested_position,
                });
            }
            submit_patches(
                submit_context.api_base_url.clone(),
                submit_context.program_id.clone(),
                submit_context.version,
                patches,
                submit_context.generation,
                submit_context.status,
            );
            submit_context.editing_menu.set(None);
        },
            div { class: "aio-menu-table__editor-field",
                label { r#for: "menu-name-{menu_id}", "标识" }
                input { id: "menu-name-{menu_id}", name: "name", class: "aio-input", value: menu.name }
            }
            div { class: "aio-menu-table__editor-field",
                label { r#for: "menu-title-{menu_id}", "菜单名称" }
                input { id: "menu-title-{menu_id}", name: "title", class: "aio-input", value: menu.title }
            }
            div { class: "aio-menu-table__editor-field",
                label { r#for: "menu-icon-{menu_id}", "图标" }
                input { id: "menu-icon-{menu_id}", name: "icon", class: "aio-input", value: menu.icon.unwrap_or_default(), placeholder: "图标名" }
            }
            div { class: "aio-menu-table__editor-field aio-menu-table__editor-field--sort",
                label { r#for: "menu-sort-{menu_id}", "排序" }
                input { id: "menu-sort-{menu_id}", name: "sort", class: "aio-input", r#type: "number", min: "1", max: "{sibling_count}", value: "{position + 1}" }
            }
            div { class: "aio-menu-table__editor-field",
                label { r#for: "menu-permission-{menu_id}", "权限标识" }
                select { id: "menu-permission-{menu_id}", name: "permission_id", class: "aio-input",
                    option { value: "", selected: selected_permission.is_none(), "无权限限制" }
                    for permission in context.permissions.iter() {
                        option { value: "{permission.id}", selected: selected_permission == Some(permission.id), "{permission.name}" }
                    }
                }
            }
            div { class: "aio-menu-table__editor-field",
                label { r#for: "menu-page-{menu_id}", "页面" }
                select { id: "menu-page-{menu_id}", name: "page_id", class: "aio-input",
                    option { value: "", selected: menu.page_id.is_none(), "无页面（目录）" }
                    for page in context.pages.iter() {
                        option { value: "{page.id}", selected: menu.page_id == Some(page.id), "{page.name} · {page.title}" }
                    }
                }
            }
            {menu_action_select("详情权限", "detail_access", &detail_access, &context.permissions)}
            {menu_action_select("编辑权限", "edit_access", &edit_access, &context.permissions)}
            {menu_action_select("删除权限", "delete_access", &delete_access, &context.permissions)}
            div { class: "aio-menu-table__editor-field aio-menu-table__editor-field--path",
                label { r#for: "menu-path-{menu_id}", "路由" }
                input {
                    id: "menu-path-{menu_id}",
                    name: "path",
                    class: "aio-input",
                    value: route_path,
                    disabled: route_id.is_none(),
                    placeholder: "目录节点没有路由",
                }
            }
            div { class: "aio-menu-table__editor-actions",
                Button { button_type: "submit", "保存" }
                button {
                    r#type: "button",
                    onclick: move |_| cancel_editing.set(None),
                    "取消"
                }
            }
        }
    }
}

fn menu_action_value(access: &crate::MenuActionAccess) -> String {
    match access {
        crate::MenuActionAccess::Hidden => "hidden".to_owned(),
        crate::MenuActionAccess::Public => "public".to_owned(),
        crate::MenuActionAccess::Permission { permission_id } => {
            format!("permission:{permission_id}")
        }
    }
}

fn menu_action_from_form(event: &FormEvent, name: &str) -> crate::MenuActionAccess {
    let value = form_text(event, name);
    if value == "public" {
        return crate::MenuActionAccess::Public;
    }
    value
        .strip_prefix("permission:")
        .and_then(|value| SymbolId::parse(value).ok())
        .map(|permission_id| crate::MenuActionAccess::Permission { permission_id })
        .unwrap_or(crate::MenuActionAccess::Hidden)
}

fn menu_action_select(
    title: &'static str,
    name: &'static str,
    selected: &str,
    permissions: &[PermissionDefinition],
) -> Element {
    rsx! {
        div { class: "aio-menu-table__editor-field",
            label { r#for: "{name}", "{title}" }
            select { id: "{name}", name, class: "aio-input",
                option { value: "hidden", selected: selected == "hidden", "不显示" }
                option { value: "public", selected: selected == "public", "公开" }
                for permission in permissions {
                    option {
                        value: "permission:{permission.id}",
                        selected: selected == format!("permission:{}", permission.id),
                        "{permission.name}"
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PageSettingsTab {
    #[default]
    Layout,
    Models,
    Functions,
}

#[component]
fn PageRendererSettings(
    page: PageDefinition,
    program_name: String,
    models: Vec<ModelDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut settings_open: Signal<bool>,
    selected_model: Signal<Option<SymbolId>>,
    draft: DraftSnapshot,
) -> Element {
    let page_id = page.id;
    let suggested_tree_table = suggest_user_tree_table(&page, &models);
    let initial_kind = page_renderer_key(&page.renderer).to_owned();
    let initial_table_model = page_table(&page.renderer)
        .and_then(|table| table.model_id)
        .or_else(|| suggested_tree_table.as_ref().map(|(_, table)| *table))
        .map(|id| id.to_string())
        .unwrap_or_default();
    let initial_tree_model = page_tree(&page.renderer)
        .and_then(|tree| tree.model_id)
        .or_else(|| suggested_tree_table.as_ref().map(|(tree, _)| *tree))
        .map(|id| id.to_string())
        .unwrap_or_default();
    let suggested_kind = if suggested_tree_table.is_some() {
        "tree_table"
    } else {
        &initial_kind
    };
    let mut renderer_kind = use_signal(move || suggested_kind.to_owned());
    let mut table_model = use_signal(move || initial_table_model);
    let mut tree_model = use_signal(move || initial_tree_model);
    let mut settings_tab = use_signal(PageSettingsTab::default);
    let expected_path = crate::convention_page_path(&program_name, &page.name);
    let selected_table = SymbolId::parse(&table_model()).ok();
    let selected_tree = SymbolId::parse(&tree_model()).ok();
    let table_fields = selected_table
        .and_then(|id| models.iter().find(|model| model.id == id))
        .map(|model| model.fields.clone())
        .unwrap_or_default();
    let tree_fields = selected_tree
        .and_then(|id| models.iter().find(|model| model.id == id))
        .map(|model| model.fields.clone())
        .unwrap_or_default();
    let current_table = page_table(&page.renderer).cloned().unwrap_or_default();
    let current_tree = page_tree(&page.renderer).cloned().unwrap_or_default();
    let save_api = api_base_url.clone();
    let save_application = program_id.clone();
    let models_api = api_base_url.clone();
    let functions_api = api_base_url.clone();
    let generate_api = api_base_url;
    rsx! {
        div { class: "aio-page-settings__backdrop", onclick: move |_| settings_open.set(false) }
        aside {
            class: "aio-page-settings__panel aio-page-settings__panel--fullscreen",
            aria_label: "页面设置",
            header {
                div {
                    strong { "页面设置" }
                    p { "{page.title}" }
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
                    variant: if settings_tab() == PageSettingsTab::Models {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    onclick: move |_| settings_tab.set(PageSettingsTab::Models),
                    "模型"
                }
                Button {
                    size: ButtonSize::Sm,
                    variant: if settings_tab() == PageSettingsTab::Functions {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    onclick: move |_| settings_tab.set(PageSettingsTab::Functions),
                    "功能定义"
                }
            }
            if settings_tab() == PageSettingsTab::Layout {
                form { class: "aio-page-settings__form aio-page-layout-form", onsubmit: move |event| {
                event.prevent_default();
                let renderer = renderer_from_form(&event);
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
                settings_open.set(false);
            },
                div { class: "aio-page-layout-form__content",
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
                                value: "{renderer_kind}",
                                onchange: move |event| renderer_kind.set(event.value()),
                                option { value: "convention_file", "约定文件渲染" }
                                option { value: "tree_table", "内置 · 左树右表" }
                                option { value: "crud_table", "内置 · 增删改查表格" }
                            }
                            if renderer_kind() == "convention_file" {
                                div { class: "aio-page-settings__convention",
                                    code { "{expected_path}" }
                                    p { "文件名由程序标识和页面标识自动推导，代码中无需再声明组件。" }
                                    Button {
                                        button_type: "button",
                                        variant: ButtonVariant::Outline,
                                        onclick: move |_| generate_convention_file(
                                            generate_api.clone(),
                                            page_id,
                                            status,
                                        ),
                                        "生成期望文件"
                                    }
                                }
                            } else {
                                label { r#for: "table-model", "表格模型" }
                                select {
                                    id: "table-model",
                                    name: "table_model_id",
                                    class: "aio-input",
                                    value: "{table_model}",
                                    onchange: move |event| table_model.set(event.value()),
                                    option { value: "", "选择模型" }
                                    for model in &models {
                                        option { value: "{model.id}", "{model.title} · {model.name}" }
                                    }
                                }
                                label { r#for: "page-size", "每页条数" }
                                input {
                                    id: "page-size",
                                    name: "page_size",
                                    class: "aio-input",
                                    r#type: "number",
                                    min: "1",
                                    max: "200",
                                    value: "{current_table.page_size}"
                                }
                            }
                            if renderer_kind() == "tree_table" {
                                label { r#for: "tree-model", "树模型" }
                                select {
                                    id: "tree-model",
                                    name: "tree_model_id",
                                    class: "aio-input",
                                    value: "{tree_model}",
                                    onchange: move |event| tree_model.set(event.value()),
                                    option { value: "", "选择树模型" }
                                    for model in &models {
                                        option { value: "{model.id}", "{model.title} · {model.name}" }
                                    }
                                }
                                if !tree_fields.is_empty() {
                                    {field_select(
                                        "树标题字段",
                                        "tree_label_field_id",
                                        &tree_fields,
                                        current_tree.label_field_id,
                                        false,
                                    )}
                                    {field_select(
                                        "树父级字段",
                                        "tree_parent_field_id",
                                        &tree_fields,
                                        current_tree.parent_field_id,
                                        true,
                                    )}
                                }
                                if !table_fields.is_empty() {
                                    {field_select(
                                        "表关联字段",
                                        "table_relation_field_id",
                                        &table_fields,
                                        current_tree.table_relation_field_id,
                                        false,
                                    )}
                                }
                            }
                        }
                    }
                }
                footer {
                    Button { button_type: "submit", "保存设置" }
                    Button {
                        button_type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| settings_open.set(false),
                        "取消"
                    }
                }
                }
            } else if settings_tab() == PageSettingsTab::Models {
                div { class: "aio-page-settings__functions",
                    {models_panel(
                        &draft,
                        models_api,
                        generation,
                        status,
                        selected_model,
                        selected_table,
                    )}
                }
            } else {
                div { class: "aio-page-settings__functions",
                    {endpoint_panel(
                        page.clone(),
                        &draft,
                        functions_api,
                        generation,
                        status,
                    )}
                }
            }
        }
    }
}

fn page_renderer_key(renderer: &crate::PageRendererDefinition) -> &'static str {
    match renderer {
        crate::PageRendererDefinition::ConventionFile => "convention_file",
        crate::PageRendererDefinition::TreeTable { .. } => "tree_table",
        crate::PageRendererDefinition::CrudTable { .. } => "crud_table",
    }
}

fn page_table(renderer: &crate::PageRendererDefinition) -> Option<&crate::TableDefinition> {
    match renderer {
        crate::PageRendererDefinition::ConventionFile => None,
        crate::PageRendererDefinition::TreeTable { table, .. }
        | crate::PageRendererDefinition::CrudTable { table } => Some(table),
    }
}

fn page_tree(renderer: &crate::PageRendererDefinition) -> Option<&crate::TreeDefinition> {
    match renderer {
        crate::PageRendererDefinition::TreeTable { tree, .. } => Some(tree),
        crate::PageRendererDefinition::ConventionFile
        | crate::PageRendererDefinition::CrudTable { .. } => None,
    }
}

fn renderer_from_form(event: &FormEvent) -> crate::PageRendererDefinition {
    let table = crate::TableDefinition {
        model_id: form_symbol(event, "table_model_id"),
        page_size: form_text(event, "page_size")
            .parse::<u32>()
            .ok()
            .filter(|value| (1..=200).contains(value))
            .unwrap_or(20),
    };
    match form_text(event, "renderer_kind").as_str() {
        "tree_table" => crate::PageRendererDefinition::TreeTable {
            tree: crate::TreeDefinition {
                model_id: form_symbol(event, "tree_model_id"),
                label_field_id: form_symbol(event, "tree_label_field_id"),
                parent_field_id: form_symbol(event, "tree_parent_field_id"),
                table_relation_field_id: form_symbol(event, "table_relation_field_id"),
            },
            table,
        },
        "crud_table" => crate::PageRendererDefinition::CrudTable { table },
        _ => crate::PageRendererDefinition::ConventionFile,
    }
}

fn form_symbol(event: &FormEvent, name: &str) -> Option<SymbolId> {
    SymbolId::parse(&form_text(event, name)).ok()
}

fn field_select(
    title: &'static str,
    name: &'static str,
    fields: &[FieldDefinition],
    selected: Option<SymbolId>,
    optional: bool,
) -> Element {
    rsx! {
        label { "{title}" }
        select { name, class: "aio-input",
            if optional {
                option { value: "", selected: selected.is_none(), "无父级字段" }
            } else {
                option { value: "", selected: selected.is_none(), "选择字段" }
            }
            for field in fields {
                option {
                    value: "{field.id}",
                    selected: selected == Some(field.id),
                    "{field.title} · {field.name}"
                }
            }
        }
    }
}

fn generate_convention_file(
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

/// 为用户管理页面推断部门树与用户表，避免用户手动拼装常见布局。
fn suggest_user_tree_table(
    page: &PageDefinition,
    models: &[ModelDefinition],
) -> Option<(SymbolId, SymbolId)> {
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
    Some((tree.id, table.id))
}

fn endpoint_panel(
    page: PageDefinition,
    draft: &DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let compiled_page = crate::compile_page(&draft.definition, &page);
    let built_in_endpoints = compiled_page
        .endpoints
        .into_iter()
        .filter(|endpoint| endpoint.source == PageEndpointSource::BuiltIn)
        .collect::<Vec<_>>();
    let custom_endpoints = page.endpoints.clone();
    let page_id = page.id;
    let endpoint_count = custom_endpoints.len();
    let version = draft.version;
    let program_id = draft.program_id.clone();
    let create_api = api_base_url.clone();
    let create_program = program_id.clone();
    let ai_api = api_base_url.clone();
    let page_name = page.name.clone();
    let page_title = page.title.clone();
    rsx! {
        section { class: "aio-endpoint-workbench",
            header { class: "aio-endpoint-workbench__header",
                div {
                    h2 { "接口目录" }
                    p { "{page.title}" }
                }
                Button {
                    onclick: move |_| {
                        let endpoint = PageEndpointDefinition {
                            id: SymbolId::new(),
                            name: format!("custom_endpoint_{}", endpoint_count + 1),
                            title: "自定义接口".to_owned(),
                            state: DefinitionState::Known,
                            intent: String::new(),
                            method: RestMethod::Post,
                            path: format!("/api/{page_name}/custom-endpoint-{}", endpoint_count + 1),
                            inputs: Vec::new(),
                            outputs: Vec::new(),
                        };
                        submit_patches(
                            create_api.clone(),
                            create_program.clone(),
                            version,
                            vec![GraphPatch::Insert {
                                parent_id: page_id,
                                collection: ChildCollection::PageEndpoints,
                                index: endpoint_count,
                                entity: GraphEntity::PageEndpoint(endpoint),
                            }],
                            generation,
                            status,
                        );
                    },
                    icons::Plus { class: "size-4" }
                    "新增接口"
                }
            }
            if !built_in_endpoints.is_empty() {
                section { class: "aio-endpoint-section",
                    div { class: "aio-endpoint-section__title",
                        h3 { "内置接口" }
                        Badge { variant: BadgeVariant::Outline, "{built_in_endpoints.len()}" }
                    }
                    div { class: "aio-endpoint-catalog",
                        for endpoint in built_in_endpoints {
                            article { class: "aio-endpoint-catalog__item",
                                span { class: method_class(endpoint.method), "{endpoint.method.as_str()}" }
                                div {
                                    strong { "{endpoint.title}" }
                                    code { "{endpoint.path}" }
                                }
                                code { class: "aio-endpoint-catalog__provider", "{short_provider_key(&endpoint.route_instruction.provider_key)}" }
                            }
                        }
                    }
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
                    textarea {
                        name: "endpoint_intent",
                        class: "aio-input",
                        rows: "3",
                        placeholder: "例如：按部门批量停用用户，返回成功数量和失败用户 ID"
                    }
                    Button { button_type: "submit",
                        icons::Sparkles { class: "size-4" }
                        "生成 REST 元数据"
                    }
                }
            }
            section { class: "aio-endpoint-section",
                div { class: "aio-endpoint-section__title",
                    h3 { "自定义接口" }
                    Badge { variant: BadgeVariant::Outline, "{custom_endpoints.len()}" }
                }
                if custom_endpoints.is_empty() {
                    {empty_panel("暂无自定义接口")}
                } else {
                    div { class: "aio-endpoint-editor-list",
                        for endpoint in custom_endpoints {
                            {endpoint_editor(
                                endpoint,
                                api_base_url.clone(),
                                program_id.clone(),
                                version,
                                generation,
                                status,
                            )}
                        }
                    }
                }
            }
        }
    }
}

fn endpoint_editor(
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let endpoint_id = endpoint.id;
    let save_endpoint = endpoint.clone();
    let save_api = api_base_url.clone();
    let save_program = program_id.clone();
    let add_input_endpoint = endpoint.clone();
    let add_input_api = api_base_url.clone();
    let add_input_program = program_id.clone();
    let add_output_endpoint = endpoint.clone();
    let add_output_api = api_base_url.clone();
    let add_output_program = program_id.clone();
    let delete_api = api_base_url;
    let delete_program = program_id;
    rsx! {
        form { class: "aio-endpoint-editor", onsubmit: move |event| {
            event.prevent_default();
            let name = form_text(&event, "name").trim().to_owned();
            let title = form_text(&event, "title").trim().to_owned();
            let path = form_text(&event, "path").trim().to_owned();
            if name.is_empty() || title.is_empty() || !path.starts_with('/') {
                let mut status = status;
                status.set(Some("接口标识、标题不能为空，路径必须以 / 开头".to_owned()));
                return;
            }
            let inputs = save_endpoint
                .inputs
                .iter()
                .map(|input| endpoint_input_from_form(&event, input))
                .collect();
            let outputs = save_endpoint
                .outputs
                .iter()
                .map(|output| endpoint_output_from_form(&event, output))
                .collect();
            let updated = PageEndpointDefinition {
                id: save_endpoint.id,
                name,
                title,
                state: save_endpoint.state.clone(),
                intent: form_text(&event, "intent").trim().to_owned(),
                method: rest_method_from_key(&form_text(&event, "method")),
                path,
                inputs,
                outputs,
            };
            submit_endpoint_update(
                updated,
                save_api.clone(),
                save_program.clone(),
                version,
                generation,
                status,
            );
        },
            header { class: "aio-endpoint-editor__header",
                div { class: "aio-endpoint-request-line",
                    select { name: "method", class: "aio-input aio-endpoint-method",
                        {rest_method_options(endpoint.method)}
                    }
                    input {
                        name: "path",
                        class: "aio-input aio-endpoint-path",
                        value: "{endpoint.path}",
                        placeholder: "/api/users/batch-disable"
                    }
                }
                Button {
                    button_type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "删除接口",
                    aria_label: "删除接口",
                    onclick: move |_| submit_patches(
                        delete_api.clone(),
                        delete_program.clone(),
                        version,
                        vec![GraphPatch::Delete { target_id: endpoint_id }],
                        generation,
                        status,
                    ),
                    icons::Trash2 { class: "size-4" }
                }
            }
            div { class: "aio-endpoint-editor__identity",
                label { "接口标识"
                    input { name: "name", class: "aio-input", value: "{endpoint.name}" }
                }
                label { "显示名称"
                    input { name: "title", class: "aio-input", value: "{endpoint.title}" }
                }
            }
            label { class: "aio-endpoint-editor__intent", "中文需求"
                textarea { name: "intent", class: "aio-input", rows: "2", value: "{endpoint.intent}" }
            }
            section { class: "aio-endpoint-parameters",
                header {
                    strong { "入参" }
                    Button {
                        button_type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| {
                            let mut updated = add_input_endpoint.clone();
                            let index = updated.inputs.len() + 1;
                            updated.inputs.push(EndpointInputDefinition {
                                id: SymbolId::new(),
                                name: format!("input_{index}"),
                                title: format!("入参 {index}"),
                                location: EndpointInputLocation::Body,
                                value_type: ValueType::Text,
                                required: false,
                            });
                            submit_endpoint_update(
                                updated,
                                add_input_api.clone(),
                                add_input_program.clone(),
                                version,
                                generation,
                                status,
                            );
                        },
                        icons::Plus { class: "size-4" }
                        "入参"
                    }
                }
                if endpoint.inputs.is_empty() {
                    div { class: "aio-endpoint-parameters__empty", "无入参" }
                } else {
                    div { class: "aio-endpoint-parameter-table",
                        div { class: "aio-endpoint-parameter-table__head",
                            span { "名称" }
                            span { "说明" }
                            span { "位置" }
                            span { "类型" }
                            span { "必填" }
                            span {}
                        }
                        for input in &endpoint.inputs {
                            {endpoint_input_row(
                                input.clone(),
                                endpoint.clone(),
                                save_api.clone(),
                                save_program.clone(),
                                version,
                                generation,
                                status,
                            )}
                        }
                    }
                }
            }
            section { class: "aio-endpoint-parameters",
                header {
                    strong { "响应 data" }
                    Button {
                        button_type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| {
                            let mut updated = add_output_endpoint.clone();
                            let index = updated.outputs.len() + 1;
                            updated.outputs.push(EndpointOutputDefinition {
                                id: SymbolId::new(),
                                name: format!("output_{index}"),
                                title: format!("出参 {index}"),
                                value_type: ValueType::Text,
                            });
                            submit_endpoint_update(
                                updated,
                                add_output_api.clone(),
                                add_output_program.clone(),
                                version,
                                generation,
                                status,
                            );
                        },
                        icons::Plus { class: "size-4" }
                        "出参"
                    }
                }
                if endpoint.outputs.is_empty() {
                    div { class: "aio-endpoint-parameters__empty", "无响应字段" }
                } else {
                    div { class: "aio-endpoint-output-grid",
                        for output in &endpoint.outputs {
                            {endpoint_output_row(
                                output.clone(),
                                endpoint.clone(),
                                save_api.clone(),
                                save_program.clone(),
                                version,
                                generation,
                                status,
                            )}
                        }
                    }
                }
            }
            footer {
                Button { button_type: "submit",
                    icons::Save { class: "size-4" }
                    "保存接口"
                }
            }
        }
    }
}

fn endpoint_input_row(
    input: EndpointInputDefinition,
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let name_field = endpoint_input_field_name(input.id, "name");
    let title_field = endpoint_input_field_name(input.id, "title");
    let location_field = endpoint_input_field_name(input.id, "location");
    let type_field = endpoint_input_field_name(input.id, "type");
    let required_field = endpoint_input_field_name(input.id, "required");
    let input_id = input.id;
    rsx! {
        div { class: "aio-endpoint-parameter-table__row",
            input { name: name_field, class: "aio-input", value: "{input.name}" }
            input { name: title_field, class: "aio-input", value: "{input.title}" }
            select { name: location_field, class: "aio-input",
                {endpoint_location_options(input.location)}
            }
            {endpoint_value_type_select(type_field, &input.value_type)}
            input {
                name: required_field,
                r#type: "checkbox",
                value: "true",
                checked: input.required,
                aria_label: "必填"
            }
            Button {
                button_type: "button",
                size: ButtonSize::IconSm,
                variant: ButtonVariant::Ghost,
                title: "删除入参",
                aria_label: "删除入参",
                onclick: move |_| {
                    let mut updated = endpoint.clone();
                    updated.inputs.retain(|value| value.id != input_id);
                    submit_endpoint_update(
                        updated,
                        api_base_url.clone(),
                        program_id.clone(),
                        version,
                        generation,
                        status,
                    );
                },
                icons::X { class: "size-4" }
            }
        }
    }
}

fn endpoint_output_row(
    output: EndpointOutputDefinition,
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let name_field = endpoint_output_field_name(output.id, "name");
    let title_field = endpoint_output_field_name(output.id, "title");
    let type_field = endpoint_output_field_name(output.id, "type");
    let output_id = output.id;
    rsx! {
        div { class: "aio-endpoint-output-grid__row",
            input { name: name_field, class: "aio-input", value: "{output.name}" }
            input { name: title_field, class: "aio-input", value: "{output.title}" }
            {endpoint_value_type_select(type_field, &output.value_type)}
            Button {
                button_type: "button",
                size: ButtonSize::IconSm,
                variant: ButtonVariant::Ghost,
                title: "删除出参",
                aria_label: "删除出参",
                onclick: move |_| {
                    let mut updated = endpoint.clone();
                    updated.outputs.retain(|value| value.id != output_id);
                    submit_endpoint_update(
                        updated,
                        api_base_url.clone(),
                        program_id.clone(),
                        version,
                        generation,
                        status,
                    );
                },
                icons::X { class: "size-4" }
            }
        }
    }
}

fn endpoint_input_from_form(
    event: &FormEvent,
    input: &EndpointInputDefinition,
) -> EndpointInputDefinition {
    EndpointInputDefinition {
        id: input.id,
        name: form_text(event, &endpoint_input_field_name(input.id, "name"))
            .trim()
            .to_owned(),
        title: form_text(event, &endpoint_input_field_name(input.id, "title"))
            .trim()
            .to_owned(),
        location: endpoint_location_from_key(&form_text(
            event,
            &endpoint_input_field_name(input.id, "location"),
        )),
        value_type: editable_value_type_from_key(
            &form_text(event, &endpoint_input_field_name(input.id, "type")),
            &input.value_type,
        ),
        required: !form_text(event, &endpoint_input_field_name(input.id, "required")).is_empty(),
    }
}

fn endpoint_output_from_form(
    event: &FormEvent,
    output: &EndpointOutputDefinition,
) -> EndpointOutputDefinition {
    EndpointOutputDefinition {
        id: output.id,
        name: form_text(event, &endpoint_output_field_name(output.id, "name"))
            .trim()
            .to_owned(),
        title: form_text(event, &endpoint_output_field_name(output.id, "title"))
            .trim()
            .to_owned(),
        value_type: editable_value_type_from_key(
            &form_text(event, &endpoint_output_field_name(output.id, "type")),
            &output.value_type,
        ),
    }
}

fn submit_endpoint_update(
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) {
    let endpoint_id = endpoint.id;
    let value = match serde_json::to_value(endpoint) {
        Ok(value) => value,
        Err(error) => {
            status.set(Some(format!("序列化接口失败: {error}")));
            return;
        }
    };
    submit_patches(
        api_base_url,
        program_id,
        version,
        vec![GraphPatch::SetProperty {
            target_id: endpoint_id,
            property: crate::EditableProperty::PageEndpoint,
            value,
        }],
        generation,
        status,
    );
}

fn generate_endpoint_with_ai(
    api_base_url: String,
    page_id: SymbolId,
    page_title: String,
    version: i64,
    intent: String,
    mut generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) {
    spawn(async move {
        let prompt = format!(
            "只为页面 {page_title}（SymbolId: {page_id}）新增一个自定义 REST 接口。\
             必须使用 GraphPatch::Insert，parent_id 为该页面，collection 为 page_endpoints，\
             entity 为 page_endpoint。根据中文需求生成 snake_case 接口标识、中文标题、HTTP 方法、\
             本应用相对路径、完整 inputs 和 outputs；路径参数必须在 path 中使用同名花括号。\
             intent 原样保存。中文需求：{intent}"
        );
        let request = VibeRunRequest {
            prompt,
            model: None,
        };
        match post_api::<_, VibeRunAccepted>(
            &api_base_url,
            "/api/studio/program/vibe-runs",
            &request,
        )
        .await
        {
            Ok(_) => status.set(Some("正在生成接口元数据".to_owned())),
            Err(error) => {
                status.set(Some(error));
                return;
            }
        }
        for _ in 0..60 {
            TimeoutFuture::new(1_000).await;
            match get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await {
                Ok(draft) if draft.version > version => {
                    generation.with_mut(|value| *value = value.saturating_add(1));
                    status.set(Some("接口元数据已生成".to_owned()));
                    return;
                }
                Ok(_) | Err(_) => {}
            }
        }
        status.set(Some("接口仍在生成，可稍后重新打开页面设置查看".to_owned()));
    });
}

fn rest_method_options(selected: RestMethod) -> Element {
    rsx! {
        for method in [RestMethod::Get, RestMethod::Post, RestMethod::Put, RestMethod::Patch, RestMethod::Delete] {
            option { value: method.as_str(), selected: method == selected, "{method.as_str()}" }
        }
    }
}

fn rest_method_from_key(value: &str) -> RestMethod {
    match value {
        "GET" => RestMethod::Get,
        "PUT" => RestMethod::Put,
        "PATCH" => RestMethod::Patch,
        "DELETE" => RestMethod::Delete,
        _ => RestMethod::Post,
    }
}

fn endpoint_location_options(selected: EndpointInputLocation) -> Element {
    rsx! {
        option { value: "path", selected: selected == EndpointInputLocation::Path, "Path" }
        option { value: "query", selected: selected == EndpointInputLocation::Query, "Query" }
        option { value: "header", selected: selected == EndpointInputLocation::Header, "Header" }
        option { value: "body", selected: selected == EndpointInputLocation::Body, "Body" }
    }
}

fn endpoint_location_from_key(value: &str) -> EndpointInputLocation {
    match value {
        "path" => EndpointInputLocation::Path,
        "query" => EndpointInputLocation::Query,
        "header" => EndpointInputLocation::Header,
        _ => EndpointInputLocation::Body,
    }
}

fn endpoint_value_type_select(name: String, value_type: &ValueType) -> Element {
    let selected = editable_value_type_key(value_type);
    rsx! {
        select { name, class: "aio-input",
            if selected == "preserve" {
                option { value: "preserve", selected: true, "{value_type_label(value_type)}" }
            }
            option { value: "text", selected: selected == "text", "文本" }
            option { value: "integer", selected: selected == "integer", "整数" }
            option { value: "decimal", selected: selected == "decimal", "小数" }
            option { value: "boolean", selected: selected == "boolean", "布尔" }
            option { value: "timestamp_ms", selected: selected == "timestamp_ms", "时间" }
            option { value: "file", selected: selected == "file", "文件" }
            option { value: "any", selected: selected == "any", "任意结构" }
        }
    }
}

fn endpoint_input_field_name(id: SymbolId, field: &str) -> String {
    format!("input_{id}_{field}")
}

fn endpoint_output_field_name(id: SymbolId, field: &str) -> String {
    format!("output_{id}_{field}")
}

fn method_class(method: RestMethod) -> &'static str {
    match method {
        RestMethod::Get => "aio-http-method aio-http-method--get",
        RestMethod::Post => "aio-http-method aio-http-method--post",
        RestMethod::Put | RestMethod::Patch => "aio-http-method aio-http-method--write",
        RestMethod::Delete => "aio-http-method aio-http-method--delete",
    }
}

fn short_provider_key(provider_key: &str) -> &str {
    provider_key
        .rsplit_once("::")
        .map_or(provider_key, |(_, name)| name)
}

fn models_panel(
    draft: &DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut selected_model: Signal<Option<SymbolId>>,
    preferred_model_id: Option<SymbolId>,
) -> Element {
    let storage_id = draft.program_id.clone();
    let root_id = draft.definition.id;
    let version = draft.version;
    let count = draft.definition.models.len();
    let current_model_id = selected_model()
        .filter(|selected_id| {
            draft
                .definition
                .models
                .iter()
                .any(|model| model.id == *selected_id)
        })
        .or_else(|| {
            preferred_model_id.filter(|preferred_id| {
                draft
                    .definition
                    .models
                    .iter()
                    .any(|model| model.id == *preferred_id)
            })
        })
        .or_else(|| draft.definition.models.first().map(|model| model.id));
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
    let create_api = api_base_url.clone();
    let create_program_id = storage_id.clone();
    rsx! {
        section { class: "aio-model-designer",
            header { class: "aio-model-designer__header",
                div {
                    h2 { "模型定义" }
                    p { "{count} 个模型" }
                }
                Button { onclick: move |_| {
                    let model_id = SymbolId::new();
                    let suffix = model_id.to_string().replace('-', "");
                    selected_model.set(Some(model_id));
                    submit_patches(
                        create_api.clone(), create_program_id.clone(), version,
                        vec![GraphPatch::Insert {
                            parent_id: root_id,
                            collection: ChildCollection::Models,
                            index: count,
                            entity: GraphEntity::Model(ModelDefinition {
                                id: model_id,
                                name: format!("model_{}", &suffix[..8]),
                                title: format!("模型 {}", count + 1),
                                state: DefinitionState::Known,
                                fields: Vec::new(),
                                indexes: Vec::new(),
                            }),
                        }], generation, status,
                    );
                },
                    icons::Plus { class: "size-4" }
                    "新建模型"
                }
            }
            div { class: "aio-model-workspace",
                nav { class: "aio-model-workspace__directory", aria_label: "模型目录",
                    div { class: "aio-model-workspace__directory-heading", "模型目录" }
                    div { class: "aio-model-workspace__directory-list",
                        for model in &draft.definition.models {
                            button {
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
                            key: "{model.id}:{version}",
                            model,
                            api_base_url: api_base_url.clone(),
                            program_id: storage_id.clone(),
                            version,
                            generation,
                            status,
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
        }
    }
}

fn copy_json_to_clipboard(json: String, mut status: Signal<Option<String>>) {
    #[cfg(target_arch = "wasm32")]
    spawn(async move {
        let Some(window) = web_sys::window() else {
            status.set(Some("无法访问浏览器剪贴板".to_owned()));
            return;
        };
        let result =
            wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(&json))
                .await;
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

#[component]
fn ModelGrid(
    model: ModelDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let model_id = model.id;
    let field_count = model.fields.len();
    let index_count = model.indexes.len();
    let fields = model.fields.clone();
    let indexes = model.indexes.clone();
    let initial_model_name = model.name.clone();
    let initial_model_title = model.title.clone();
    let mut model_name = use_signal(move || initial_model_name);
    let mut model_title = use_signal(move || initial_model_title);
    let save_api = api_base_url.clone();
    let save_program_id = program_id.clone();
    rsx! {
        section { class: "aio-model-grid",
            div { class: "aio-model-grid__identity",
                label {
                    span { "模型标识" }
                    input {
                        class: "aio-input",
                        aria_label: "模型标识",
                        value: model_name(),
                        oninput: move |event| model_name.set(event.value()),
                    }
                }
                label {
                    span { "模型标题" }
                    input {
                        class: "aio-input",
                        aria_label: "模型标题",
                        value: model_title(),
                        oninput: move |event| model_title.set(event.value()),
                    }
                }
                div { class: "aio-model-grid__metrics",
                    span { strong { "{field_count}" } "字段" }
                    span { strong { "{index_count}" } "索引" }
                }
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "保存模型",
                    aria_label: "保存模型",
                    onclick: move |_| {
                        let name = model_name().trim().to_owned();
                        let title = model_title().trim().to_owned();
                        if name.is_empty() || title.is_empty() {
                            status.set(Some("模型标识和标题不能为空".to_owned()));
                            return;
                        }
                        submit_patches(
                            save_api.clone(), save_program_id.clone(), version,
                            vec![GraphPatch::Rename {
                                target_id: model_id,
                                name,
                                title: Some(title),
                            }], generation, status,
                        );
                    },
                    icons::Save { class: "size-4" }
                }
            }
            div { class: "aio-model-grid__section-heading",
                h3 { "字段" }
                span { "{field_count} 项" }
            }
            div { class: "aio-edit-grid aio-edit-grid--fields",
                table {
                    thead { tr {
                        th { "字段标识" }
                        th { "字段标题" }
                        th { "类型" }
                        th { class: "aio-edit-grid__toggle", "必填" }
                        th { class: "aio-edit-grid__toggle", "列表" }
                        th { class: "aio-edit-grid__toggle", "详情" }
                        th { class: "aio-edit-grid__toggle", "表单" }
                        th { class: "aio-edit-grid__toggle", "编辑" }
                        th { class: "aio-edit-grid__toggle", "查询" }
                        th { class: "aio-edit-grid__toggle", "排序" }
                        th { class: "aio-edit-grid__toggle", "唯一" }
                        th { class: "aio-edit-grid__toggle", "导入" }
                        th { class: "aio-edit-grid__toggle", "导出" }
                        th { class: "aio-edit-grid__toggle", "AI" }
                        th { "默认值" }
                        th { "占位提示" }
                        th { "帮助文本" }
                        th { "校验规则" }
                        th { class: "aio-edit-grid__actions", "操作" }
                    } }
                    tbody {
                        for field in &fields {
                            FieldGridRow {
                                key: "{field.id}:{version}",
                                field: field.clone(),
                                api_base_url: api_base_url.clone(),
                                program_id: program_id.clone(),
                                version,
                                generation,
                                status,
                            }
                        }
                        NewFieldGridRow {
                            key: "new-field:{model_id}:{version}",
                            model_id,
                            field_count,
                            api_base_url: api_base_url.clone(),
                            program_id: program_id.clone(),
                            version,
                            generation,
                            status,
                        }
                    }
                }
            }
            div { class: "aio-model-grid__section-heading",
                h3 { "表达式索引" }
                span { "{index_count} 项" }
            }
            div { class: "aio-edit-grid aio-edit-grid--indexes",
                table {
                    thead { tr {
                        th { "索引字段" }
                        th { "用途" }
                        th { class: "aio-edit-grid__actions", "操作" }
                    } }
                    tbody {
                        for index in &indexes {
                            IndexGridRow {
                                key: "{index.id}:{version}",
                                index: index.clone(),
                                fields: fields.clone(),
                                api_base_url: api_base_url.clone(),
                                program_id: program_id.clone(),
                                version,
                                generation,
                                status,
                            }
                        }
                        NewIndexGridRow {
                            key: "new-index:{model_id}:{version}",
                            model_id,
                            index_count,
                            fields,
                            api_base_url,
                            program_id,
                            version,
                            generation,
                            status,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FieldGridRow(
    field: FieldDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let field_id = field.id;
    let current_value_type = field.value_type.clone();
    let initial_name = field.name.clone();
    let initial_title = field.title.clone();
    let initial_type = editable_value_type_key(&field.value_type).to_owned();
    let initial_required = field.required;
    let initial_options = field.options.clone();
    let initial_default_value = field
        .options
        .default_value
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    let initial_placeholder = field.options.placeholder.clone().unwrap_or_default();
    let initial_help_text = field.options.help_text.clone().unwrap_or_default();
    let initial_validation =
        serde_json::to_string(&field.options.validation).unwrap_or_else(|_| "{}".to_owned());
    let mut name = use_signal(move || initial_name);
    let mut title = use_signal(move || initial_title);
    let mut value_type = use_signal(move || initial_type);
    let mut required = use_signal(move || initial_required);
    let options = use_signal(move || initial_options);
    let mut default_value = use_signal(move || initial_default_value);
    let mut placeholder = use_signal(move || initial_placeholder);
    let mut help_text = use_signal(move || initial_help_text);
    let mut validation = use_signal(move || initial_validation);
    rsx! {
        tr { "data-field-id": "{field_id}",
            td { input {
                aria_label: "字段标识 {field.name}",
                value: name(),
                oninput: move |event| name.set(event.value()),
            } }
            td { input {
                aria_label: "字段标题 {field.name}",
                value: title(),
                oninput: move |event| title.set(event.value()),
            } }
            td { select {
                aria_label: "字段类型 {field.name}",
                onchange: move |event| value_type.set(event.value()),
                {editable_value_type_options(&current_value_type, value_type())}
            } }
            td { class: "aio-edit-grid__toggle", input {
                aria_label: "字段必填 {field.name}",
                r#type: "checkbox",
                checked: required(),
                onchange: move |event| required.set(event.checked()),
            } }
            FieldOptionCells { options, field_label: field.title.clone() }
            td { input {
                aria_label: "默认值 {field.name}",
                placeholder: "JSON 或文本",
                value: default_value(),
                oninput: move |event| default_value.set(event.value()),
            } }
            td { input {
                aria_label: "占位提示 {field.name}",
                value: placeholder(),
                oninput: move |event| placeholder.set(event.value()),
            } }
            td { input {
                aria_label: "帮助文本 {field.name}",
                value: help_text(),
                oninput: move |event| help_text.set(event.value()),
            } }
            td { input {
                aria_label: "校验规则 {field.name}",
                title: "JSON: min_length, max_length, minimum, maximum, pattern",
                placeholder: "{{}}",
                value: validation(),
                oninput: move |event| validation.set(event.value()),
            } }
            td { class: "aio-edit-grid__actions",
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "保存字段 {field.title}",
                    aria_label: "保存字段 {field.name}",
                    onclick: move |_| {
                        let next_name = name().trim().to_owned();
                        let next_title = title().trim().to_owned();
                        if next_name.is_empty() || next_title.is_empty() {
                            status.set(Some("字段标识和标题不能为空".to_owned()));
                            return;
                        }
                        let next_value_type = editable_value_type_from_key(
                            &value_type(),
                            &current_value_type,
                        );
                        let next_validation = match serde_json::from_str::<crate::FieldValidation>(
                            validation().trim(),
                        ) {
                            Ok(value) => value,
                            Err(error) => {
                                status.set(Some(format!("校验规则必须是合法 JSON: {error}")));
                                return;
                            }
                        };
                        let mut next_options = options();
                        next_options.default_value = editable_default_value(&default_value());
                        next_options.placeholder = non_empty_text(&placeholder());
                        next_options.help_text = non_empty_text(&help_text());
                        next_options.validation = next_validation;
                        submit_patches(
                            api_base_url.clone(), program_id.clone(), version,
                            vec![
                                GraphPatch::Rename {
                                    target_id: field_id,
                                    name: next_name,
                                    title: Some(next_title),
                                },
                                GraphPatch::SetProperty {
                                    target_id: field_id,
                                    property: crate::EditableProperty::FieldValueType,
                                    value: serde_json::json!(next_value_type),
                                },
                                GraphPatch::SetProperty {
                                    target_id: field_id,
                                    property: crate::EditableProperty::FieldRequired,
                                    value: serde_json::Value::Bool(required()),
                                },
                                GraphPatch::SetProperty {
                                    target_id: field_id,
                                    property: crate::EditableProperty::FieldOptions,
                                    value: serde_json::json!(next_options),
                                },
                            ],
                            generation,
                            status,
                        );
                    },
                    icons::Save { class: "size-4" }
                }
            }
        }
    }
}

#[component]
fn NewFieldGridRow(
    model_id: SymbolId,
    field_count: usize,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let mut name = use_signal(String::new);
    let mut title = use_signal(String::new);
    let mut value_type = use_signal(|| "text".to_owned());
    let mut required = use_signal(|| false);
    let options = use_signal(crate::FieldOptions::default);
    let mut default_value = use_signal(String::new);
    let mut placeholder = use_signal(String::new);
    let mut help_text = use_signal(String::new);
    let mut validation = use_signal(|| "{}".to_owned());
    rsx! {
        tr { class: "aio-edit-grid__new-row",
            td { input {
                aria_label: "新字段标识",
                placeholder: "新增字段标识",
                value: name(),
                oninput: move |event| name.set(event.value()),
            } }
            td { input {
                aria_label: "新字段标题",
                placeholder: "新增字段标题",
                value: title(),
                oninput: move |event| title.set(event.value()),
            } }
            td { select {
                aria_label: "新字段类型",
                onchange: move |event| value_type.set(event.value()),
                {editable_value_type_options(&ValueType::Text, value_type())}
            } }
            td { class: "aio-edit-grid__toggle", input {
                aria_label: "新字段必填",
                r#type: "checkbox",
                checked: required(),
                onchange: move |event| required.set(event.checked()),
            } }
            FieldOptionCells { options, field_label: "新字段".to_owned() }
            td { input {
                aria_label: "新字段默认值",
                placeholder: "JSON 或文本",
                value: default_value(),
                oninput: move |event| default_value.set(event.value()),
            } }
            td { input {
                aria_label: "新字段占位提示",
                value: placeholder(),
                oninput: move |event| placeholder.set(event.value()),
            } }
            td { input {
                aria_label: "新字段帮助文本",
                value: help_text(),
                oninput: move |event| help_text.set(event.value()),
            } }
            td { input {
                aria_label: "新字段校验规则",
                title: "JSON: min_length, max_length, minimum, maximum, pattern",
                placeholder: "{{}}",
                value: validation(),
                oninput: move |event| validation.set(event.value()),
            } }
            td { class: "aio-edit-grid__actions",
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "添加字段",
                    aria_label: "添加字段",
                    onclick: move |_| {
                        let next_name = name().trim().to_owned();
                        let next_title = title().trim().to_owned();
                        if next_name.is_empty() || next_title.is_empty() {
                            status.set(Some("新字段标识和标题不能为空".to_owned()));
                            return;
                        }
                        let next_validation = match serde_json::from_str::<crate::FieldValidation>(
                            validation().trim(),
                        ) {
                            Ok(value) => value,
                            Err(error) => {
                                status.set(Some(format!("校验规则必须是合法 JSON: {error}")));
                                return;
                            }
                        };
                        let mut next_options = options();
                        next_options.default_value = editable_default_value(&default_value());
                        next_options.placeholder = non_empty_text(&placeholder());
                        next_options.help_text = non_empty_text(&help_text());
                        next_options.validation = next_validation;
                        submit_patches(
                            api_base_url.clone(), program_id.clone(), version,
                            vec![GraphPatch::Insert {
                                parent_id: model_id,
                                collection: ChildCollection::Fields,
                                index: field_count,
                                entity: GraphEntity::Field(FieldDefinition {
                                    id: SymbolId::new(),
                                    name: next_name,
                                    title: next_title,
                                    value_type: editable_value_type_from_key(
                                        &value_type(),
                                        &ValueType::Text,
                                    ),
                                    state: DefinitionState::Known,
                                    required: required(),
                                    options: next_options,
                                    relation_model_id: None,
                                }),
                            }],
                            generation,
                            status,
                        );
                    },
                    icons::Plus { class: "size-4" }
                }
            }
        }
    }
}

#[component]
fn FieldOptionCells(options: Signal<crate::FieldOptions>, field_label: String) -> Element {
    let mut list_options = options;
    let mut detail_options = options;
    let mut form_options = options;
    let mut edit_options = options;
    let mut filter_options = options;
    let mut sort_options = options;
    let mut unique_options = options;
    let mut import_options = options;
    let mut export_options = options;
    let mut ai_options = options;
    rsx! {
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "列表显示 {field_label}", r#type: "checkbox",
            checked: options().list_visible,
            onchange: move |event| list_options.with_mut(|value| value.list_visible = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "详情显示 {field_label}", r#type: "checkbox",
            checked: options().detail_visible,
            onchange: move |event| detail_options.with_mut(|value| value.detail_visible = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "表单显示 {field_label}", r#type: "checkbox",
            checked: options().form_visible,
            onchange: move |event| form_options.with_mut(|value| value.form_visible = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "表单可编辑 {field_label}", r#type: "checkbox",
            checked: options().form_editable,
            onchange: move |event| edit_options.with_mut(|value| value.form_editable = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "允许查询 {field_label}", r#type: "checkbox",
            checked: options().filterable,
            onchange: move |event| filter_options.with_mut(|value| value.filterable = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "允许排序 {field_label}", r#type: "checkbox",
            checked: options().sortable,
            onchange: move |event| sort_options.with_mut(|value| value.sortable = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "唯一约束 {field_label}", r#type: "checkbox",
            checked: options().unique,
            onchange: move |event| unique_options.with_mut(|value| value.unique = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "Excel 导入 {field_label}", r#type: "checkbox",
            checked: options().excel_import,
            onchange: move |event| import_options.with_mut(|value| value.excel_import = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "Excel 导出 {field_label}", r#type: "checkbox",
            checked: options().excel_export,
            onchange: move |event| export_options.with_mut(|value| value.excel_export = event.checked()),
        } }
        td { class: "aio-edit-grid__toggle", input {
            aria_label: "AI 结构化提取 {field_label}", r#type: "checkbox",
            checked: options().ai_extract,
            onchange: move |event| ai_options.with_mut(|value| value.ai_extract = event.checked()),
        } }
    }
}

fn editable_default_value(value: &str) -> Option<Value> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned())))
    }
}

fn non_empty_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[component]
fn IndexGridRow(
    index: ModelIndexDefinition,
    fields: Vec<FieldDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let index_id = index.id;
    let initial_fields = index.fields.iter().copied().collect::<BTreeSet<_>>();
    let initial_purpose = index_purpose_key(&index.purpose).to_owned();
    let mut selected_fields = use_signal(move || initial_fields);
    let mut purpose = use_signal(move || initial_purpose);
    rsx! {
        tr { "data-index-id": "{index_id}",
            td { div { class: "aio-edit-grid__checks",
                for field in &fields {
                    label {
                        input {
                            r#type: "checkbox",
                            checked: selected_fields().contains(&field.id),
                            onchange: {
                                let field_id = field.id;
                                move |event| selected_fields.with_mut(|selected| {
                                    if event.checked() {
                                        selected.insert(field_id);
                                    } else {
                                        selected.remove(&field_id);
                                    }
                                })
                            },
                        }
                        span { "{field.title}" }
                    }
                }
            } }
            td { select {
                aria_label: "索引用途",
                value: purpose(),
                onchange: move |event| purpose.set(event.value()),
                option { value: "filter", "筛选" }
                option { value: "sort", "排序" }
                option { value: "relation", "关联" }
            } }
            td { class: "aio-edit-grid__actions",
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "保存索引",
                    aria_label: "保存索引",
                    onclick: move |_| {
                        let selected = selected_fields();
                        let ordered_fields = fields
                            .iter()
                            .filter(|field| selected.contains(&field.id))
                            .map(|field| field.id)
                            .collect::<Vec<_>>();
                        if ordered_fields.is_empty() {
                            status.set(Some("索引至少需要一个字段".to_owned()));
                            return;
                        }
                        submit_patches(
                            api_base_url.clone(), program_id.clone(), version,
                            vec![
                                GraphPatch::SetProperty {
                                    target_id: index_id,
                                    property: crate::EditableProperty::ModelIndexFields,
                                    value: serde_json::json!(ordered_fields),
                                },
                                GraphPatch::SetProperty {
                                    target_id: index_id,
                                    property: crate::EditableProperty::ModelIndexPurpose,
                                    value: serde_json::json!(index_purpose_from_key(&purpose())),
                                },
                            ],
                            generation,
                            status,
                        );
                    },
                    icons::Save { class: "size-4" }
                }
            }
        }
    }
}

#[component]
fn NewIndexGridRow(
    model_id: SymbolId,
    index_count: usize,
    fields: Vec<FieldDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let mut selected_fields = use_signal(BTreeSet::<SymbolId>::new);
    let mut purpose = use_signal(|| "filter".to_owned());
    let has_fields = !fields.is_empty();
    rsx! {
        tr { class: "aio-edit-grid__new-row",
            td { div { class: "aio-edit-grid__checks",
                for field in &fields {
                    label {
                        input {
                            r#type: "checkbox",
                            checked: selected_fields().contains(&field.id),
                            onchange: {
                                let field_id = field.id;
                                move |event| selected_fields.with_mut(|selected| {
                                    if event.checked() {
                                        selected.insert(field_id);
                                    } else {
                                        selected.remove(&field_id);
                                    }
                                })
                            },
                        }
                        span { "{field.title}" }
                    }
                }
                if !has_fields {
                    span { class: "aio-edit-grid__placeholder", "请先添加字段" }
                }
            } }
            td { select {
                aria_label: "新索引用途",
                value: purpose(),
                onchange: move |event| purpose.set(event.value()),
                option { value: "filter", "筛选" }
                option { value: "sort", "排序" }
                option { value: "relation", "关联" }
            } }
            td { class: "aio-edit-grid__actions",
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "添加索引",
                    aria_label: "添加索引",
                    disabled: !has_fields,
                    onclick: move |_| {
                        let selected = selected_fields();
                        let ordered_fields = fields
                            .iter()
                            .filter(|field| selected.contains(&field.id))
                            .map(|field| field.id)
                            .collect::<Vec<_>>();
                        if ordered_fields.is_empty() {
                            status.set(Some("请选择至少一个索引字段".to_owned()));
                            return;
                        }
                        submit_patches(
                            api_base_url.clone(), program_id.clone(), version,
                            vec![GraphPatch::Insert {
                                parent_id: model_id,
                                collection: ChildCollection::ModelIndexes,
                                index: index_count,
                                entity: GraphEntity::ModelIndex(ModelIndexDefinition {
                                    id: SymbolId::new(),
                                    fields: ordered_fields,
                                    purpose: index_purpose_from_key(&purpose()),
                                }),
                            }],
                            generation,
                            status,
                        );
                    },
                    icons::Plus { class: "size-4" }
                }
            }
        }
    }
}

fn editable_value_type_options(current: &ValueType, selected: String) -> Element {
    rsx! {
        option { value: "text", selected: selected == "text", "文本" }
        option { value: "integer", selected: selected == "integer", "整数" }
        option { value: "decimal", selected: selected == "decimal", "小数" }
        option { value: "boolean", selected: selected == "boolean", "布尔" }
        option { value: "timestamp_ms", selected: selected == "timestamp_ms", "时间" }
        option { value: "file", selected: selected == "file", "文件" }
        option { value: "any", selected: selected == "any", "任意结构" }
        if editable_value_type_key(current) == "preserve" {
            option {
                value: "preserve",
                selected: selected == "preserve",
                "{value_type_label(current)}（保持定义）"
            }
        }
    }
}

fn editable_value_type_key(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Text => "text",
        ValueType::Integer => "integer",
        ValueType::Decimal => "decimal",
        ValueType::Boolean => "boolean",
        ValueType::TimestampMs => "timestamp_ms",
        ValueType::File => "file",
        ValueType::Any => "any",
        ValueType::Null
        | ValueType::Object { .. }
        | ValueType::List { .. }
        | ValueType::Optional { .. } => "preserve",
    }
}

fn editable_value_type_from_key(key: &str, current: &ValueType) -> ValueType {
    if key == "preserve" {
        current.clone()
    } else {
        value_type_from_key(key)
    }
}

fn index_purpose_key(purpose: &IndexPurpose) -> &'static str {
    match purpose {
        IndexPurpose::Filter => "filter",
        IndexPurpose::Sort => "sort",
        IndexPurpose::Relation => "relation",
    }
}

fn index_purpose_from_key(key: &str) -> IndexPurpose {
    match key {
        "sort" => IndexPurpose::Sort,
        "relation" => IndexPurpose::Relation,
        _ => IndexPurpose::Filter,
    }
}

fn value_type_from_key(key: &str) -> ValueType {
    match key {
        "integer" => ValueType::Integer,
        "decimal" => ValueType::Decimal,
        "boolean" => ValueType::Boolean,
        "timestamp_ms" => ValueType::TimestampMs,
        "file" => ValueType::File,
        "any" => ValueType::Any,
        _ => ValueType::Text,
    }
}

fn value_type_label(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Any => "任意结构",
        ValueType::Null => "空值",
        ValueType::Boolean => "布尔",
        ValueType::Integer => "整数",
        ValueType::Decimal => "小数",
        ValueType::Text => "文本",
        ValueType::TimestampMs => "时间",
        ValueType::File => "文件",
        ValueType::Object { .. } => "对象",
        ValueType::List { .. } => "列表",
        ValueType::Optional { .. } => "可选",
    }
}

struct PendingStudioPatch {
    api_base_url: String,
    program_id: String,
    base_version: i64,
    patches: Vec<GraphPatch>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
}

thread_local! {
    static STUDIO_PATCH_QUEUE: RefCell<VecDeque<PendingStudioPatch>> = RefCell::new(VecDeque::new());
    static STUDIO_PATCH_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static STUDIO_PATCH_VERSIONS: RefCell<BTreeMap<String, i64>> = const { RefCell::new(BTreeMap::new()) };
}

fn submit_patches(
    api_base_url: String,
    program_id: String,
    base_version: i64,
    patches: Vec<GraphPatch>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) {
    STUDIO_PATCH_QUEUE.with(|queue| {
        queue.borrow_mut().push_back(PendingStudioPatch {
            api_base_url,
            program_id,
            base_version,
            patches,
            generation,
            status,
        });
    });
    let should_start = STUDIO_PATCH_ACTIVE.with(|active| {
        if active.get() {
            false
        } else {
            active.set(true);
            true
        }
    });
    if should_start {
        spawn(drain_studio_patch_queue());
    }
}

async fn drain_studio_patch_queue() {
    loop {
        let pending = STUDIO_PATCH_QUEUE.with(|queue| queue.borrow_mut().pop_front());
        let Some(pending) = pending else {
            STUDIO_PATCH_ACTIVE.with(|active| active.set(false));
            return;
        };
        let PendingStudioPatch {
            api_base_url,
            program_id,
            base_version,
            patches,
            mut generation,
            mut status,
        } = pending;
        let base_version = STUDIO_PATCH_VERSIONS.with(|versions| {
            versions
                .borrow()
                .get(&program_id)
                .copied()
                .map_or(base_version, |current| current.max(base_version))
        });
        let path = "/api/studio/program/draft";
        let batch = GraphPatchBatch {
            base_version,
            patches,
            origin: PatchOrigin::Studio,
        };
        match patch_api::<_, DraftSnapshot>(&api_base_url, &path, &batch).await {
            Ok(draft) => {
                STUDIO_PATCH_VERSIONS.with(|versions| {
                    versions.borrow_mut().insert(program_id, draft.version);
                });
                generation.with_mut(|value| *value = value.saturating_add(1));
                status.set(Some("已保存，等待自动发布".to_owned()));
            }
            Err(error) => {
                STUDIO_PATCH_VERSIONS.with(|versions| {
                    versions.borrow_mut().remove(&program_id);
                });
                if error.starts_with("draft version conflict") {
                    STUDIO_PATCH_QUEUE.with(|queue| {
                        queue
                            .borrow_mut()
                            .retain(|item| item.program_id != program_id);
                    });
                    status.set(Some("草稿已被其他操作更新，已刷新，请重试".to_owned()));
                } else {
                    status.set(Some(error));
                }
                generation.with_mut(|value| *value = value.saturating_add(1));
            }
        }
    }
}

fn empty_panel(message: &str) -> Element {
    rsx! { div { class: "grid min-h-48 place-items-center rounded-md border border-dashed p-6 text-sm text-muted-foreground", "{message}" } }
}

fn form_text(event: &FormEvent, name: &str) -> String {
    match event.get_first(name) {
        Some(dioxus::html::FormValue::Text(value)) => value,
        _ => String::new(),
    }
}
