use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use crate::{
    Alignment, ApplicationSummary, BooleanOperator, ChildCollection, CompareOperator,
    ComponentNode, ComponentStyle, CreateApplicationInput, DataSourceDefinition, DefinitionState,
    DraftSnapshot, EffectKind, FieldDefinition, FunctionDefinition, FunctionGraph, FunctionNode,
    FunctionNodeEditor, FunctionNodeKind, GraphEdge, GraphEntity, GraphPatch, GraphPatchBatch,
    IndexPurpose, MathOperator, MenuDefinition, ModelDefinition, ModelIndexDefinition,
    NotificationLevel, PageDefinition, PageStateDefinition, PatchOrigin, PermissionDefinition,
    PortDefinition, PropertyValue, RevisionSnapshot, RouteDefinition, SpacingToken, StudioCatalog,
    StudioPage as StudioPageData, SymbolId, ValueType, VibeRunAccepted, VibeRunRequest,
};
use crate::{
    ComponentCatalogEntry, ComponentIndex, ComponentPropertyKind, ComponentPropertySpec,
    ComponentRenderContext, ComponentShape, DynamicRenderData, DynamicRenderer,
};
use dioxus::prelude::*;

use crate::browser_http::{get_api, patch_api, post_api};
use crate::{
    design_system::{
        Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardContent, CardHeader,
        CardTitle,
    },
    workflow::{
        NODE_H, WorkflowCanvas, WorkflowDefaultNode, WorkflowEdge, WorkflowEdgeStyle,
        WorkflowMinimap, WorkflowNode, WorkflowNodeKind, WorkflowNodeWrapper, use_workflow,
    },
};
use gloo_timers::future::TimeoutFuture;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StudioTab {
    #[default]
    Scenes,
    Canvas,
    Logic,
    Models,
    Vibe,
    Diagnostics,
    Revisions,
}

#[component]
pub fn StudioPage(
    api_base_url: String,
    mut selected_application: Signal<Option<String>>,
) -> Element {
    let applications_generation = use_signal(|| 0_u64);
    let draft_generation = use_signal(|| 0_u64);
    let mut selected_page = use_signal(|| None::<SymbolId>);
    let mut selected_component = use_signal(|| None::<SymbolId>);
    let mut selected_function = use_signal(|| None::<SymbolId>);
    let editing_menu = use_signal(|| None::<SymbolId>);
    let collapsed_menus = use_signal(BTreeSet::<SymbolId>::new);
    let tab = use_signal(StudioTab::default);
    let status = use_signal(|| None::<String>);
    let components = use_hook(load_components);
    let catalog_api = api_base_url.clone();
    let studio_catalog = use_resource(move || {
        let api_base_url = catalog_api.clone();
        async move { get_api::<StudioCatalog>(&api_base_url, "/api/studio/catalog").await }
    });

    let applications_api = api_base_url.clone();
    let applications = use_resource(move || {
        let api_base_url = applications_api.clone();
        let _generation = applications_generation();
        async move {
            get_api::<StudioPageData<ApplicationSummary>>(
                &api_base_url,
                "/api/studio/applications?o=0&s=100",
            )
            .await
        }
    });
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let application_id = selected_application();
        let _generation = draft_generation();
        async move {
            let Some(application_id) = application_id else {
                return Ok(None);
            };
            get_api::<DraftSnapshot>(
                &api_base_url,
                &format!("/api/studio/applications/{application_id}/draft"),
            )
            .await
            .map(Some)
        }
    });
    let revisions_api = api_base_url.clone();
    let revisions = use_resource(move || {
        let api_base_url = revisions_api.clone();
        let application_id = selected_application();
        let _generation = draft_generation();
        async move {
            let Some(application_id) = application_id else {
                return Ok(None);
            };
            get_api::<StudioPageData<RevisionSnapshot>>(
                &api_base_url,
                &format!("/api/studio/applications/{application_id}/revisions?o=0&s=100"),
            )
            .await
            .map(Some)
        }
    });

    use_effect(move || {
        if selected_application.peek().is_none()
            && let Some(Ok(page)) = applications.read().as_ref()
            && let Some(application) = page.d.first()
        {
            selected_application.set(Some(application.id.clone()));
        }
    });
    use_effect(move || {
        if let Some(Ok(Some(draft))) = draft.read().as_ref()
            && !draft
                .definition
                .functions
                .iter()
                .any(|function| Some(function.id) == selected_function())
        {
            selected_function.set(draft.definition.functions.first().map(|value| value.id));
        }
    });
    use_effect(move || {
        if let Some(Ok(Some(draft))) = draft.read().as_ref()
            && !draft
                .definition
                .pages
                .iter()
                .any(|page| Some(page.id) == selected_page())
        {
            selected_page.set(draft.definition.pages.first().map(|page| page.id));
        }
    });
    use_effect(move || {
        if let Some(Ok(Some(draft))) = draft.read().as_ref() {
            let current_page = selected_page()
                .and_then(|id| draft.definition.pages.iter().find(|page| page.id == id))
                .or_else(|| draft.definition.pages.first());
            if let Some(page) = current_page
                && !component_contains_id(&page.root, selected_component())
            {
                selected_component.set(Some(page.root.id));
            }
        }
    });

    let application_page = applications.read().as_ref().cloned();
    let draft_snapshot = draft.read().as_ref().cloned();
    let revision_page = revisions.read().as_ref().cloned();
    let create_api = api_base_url.clone();

    rsx! {
        section { class: "aio-studio-shell grid min-h-[calc(100vh-8rem)] grid-cols-[13rem_minmax(0,1fr)] border bg-background",
            aside { class: "aio-studio-applications border-r bg-muted/20 p-3",
                div { class: "mb-3 flex items-center justify-between gap-2",
                    strong { class: "text-sm", "应用" }
                    Button {
                        size: ButtonSize::IconSm,
                        title: "新建应用",
                        aria_label: "新建应用",
                        onclick: move |_| create_application(
                            create_api.clone(),
                            applications_generation,
                            selected_application,
                            status,
                        ),
                        "+"
                    }
                }
                div { class: "space-y-1",
                    match application_page {
                        Some(Ok(page)) => rsx! { for application in page.d {
                            button {
                                class: if selected_application().as_deref() == Some(&application.id) {
                                    "w-full rounded-md bg-primary px-3 py-2 text-left text-sm text-primary-foreground"
                                } else {
                                    "w-full rounded-md px-3 py-2 text-left text-sm hover:bg-accent"
                                },
                                onclick: move |_| selected_application.set(Some(application.id.clone())),
                                span { class: "studio-application-title block truncate font-medium", "{application.title}" }
                                span { class: "studio-application-name block truncate text-xs opacity-70", "{application.name}" }
                            }
                        } },
                        Some(Err(error)) => rsx! { p { class: "text-sm text-destructive break-words", "{error}" } },
                        None => rsx! { p { class: "text-sm text-muted-foreground", "加载中" } },
                    }
                }
            }
            div { class: "min-w-0",
                header { class: "flex min-h-12 items-center gap-1 overflow-x-auto border-b px-3",
                    {tab_button("场景/菜单", StudioTab::Scenes, tab)}
                    {tab_button("页面画布", StudioTab::Canvas, tab)}
                    {tab_button("逻辑图", StudioTab::Logic, tab)}
                    {tab_button("模型", StudioTab::Models, tab)}
                    {tab_button("Vibe", StudioTab::Vibe, tab)}
                    {tab_button("诊断", StudioTab::Diagnostics, tab)}
                    {tab_button("Revision", StudioTab::Revisions, tab)}
                    div { class: "ml-auto shrink-0",
                        if let Some(message) = status() {
                            Badge { variant: BadgeVariant::Outline, "{message}" }
                        }
                    }
                }
                main { class: "min-w-0 p-4",
                    match draft_snapshot {
                        Some(Ok(Some(draft))) => match tab() {
                            StudioTab::Scenes => scenes_panel(
                                &draft,
                                api_base_url.clone(),
                                draft_generation,
                                status,
                                editing_menu,
                                collapsed_menus,
                            ),
                            StudioTab::Canvas => canvas_panel(
                                &draft,
                                selected_page,
                                selected_component,
                                components.as_ref(),
                                api_base_url.clone(),
                                draft_generation,
                                status,
                            ),
                            StudioTab::Logic => logic_panel(
                                &draft,
                                selected_function,
                                studio_catalog
                                    .read()
                                    .as_ref()
                                    .and_then(|value| value.as_ref().ok())
                                    .map(|value| value.capabilities.clone())
                                    .unwrap_or_default(),
                                api_base_url.clone(),
                                draft_generation,
                                status,
                            ),
                            StudioTab::Models => models_panel(
                                &draft,
                                api_base_url.clone(),
                                draft_generation,
                                status,
                            ),
                            StudioTab::Vibe => vibe_panel(
                                &draft,
                                api_base_url.clone(),
                                status,
                            ),
                            StudioTab::Diagnostics => diagnostics_panel(&draft),
                            StudioTab::Revisions => revisions_panel(
                                &draft,
                                revision_page.clone(),
                                api_base_url.clone(),
                                draft_generation,
                                status,
                            ),
                        },
                        Some(Ok(None)) => empty_panel("新建或选择一个应用"),
                        Some(Err(error)) => empty_panel(&error),
                        None => empty_panel("正在加载 Draft"),
                    }
                }
            }
        }
    }
}

fn load_components() -> Result<Arc<ComponentIndex>, String> {
    let mut context = rudi::Context::auto_register();
    ComponentIndex::from_context(&mut context)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

fn tab_button(label: &'static str, value: StudioTab, mut tab: Signal<StudioTab>) -> Element {
    let variant = if tab() == value {
        ButtonVariant::Secondary
    } else {
        ButtonVariant::Ghost
    };
    rsx! {
        Button {
            class: "shrink-0",
            variant,
            size: ButtonSize::Sm,
            onclick: move |_| tab.set(value),
            "{label}"
        }
    }
}

fn scenes_panel(
    draft: &DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    editing_menu: Signal<Option<SymbolId>>,
    collapsed_menus: Signal<BTreeSet<SymbolId>>,
) -> Element {
    let application_id = draft.application_id.clone();
    let version = draft.version;
    let program_id = draft.definition.id;
    let scene_count = draft.definition.menus.len();
    let page_count = draft.definition.pages.len();
    let first_scene = draft.definition.menus.first().map(|value| value.id);
    let scene_api = api_base_url.clone();
    let scene_application_id = application_id.clone();
    let page_api = api_base_url;
    let page_application_id = application_id;
    let table_context = MenuTableContext {
        api_base_url: scene_api.clone(),
        application_id: scene_application_id.clone(),
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
                    h2 { "场景与菜单" }
                }
                div { class: "aio-menu-management__actions",
                    Button {
                        onclick: move |_| {
                            let scene = MenuDefinition {
                                id: SymbolId::new(),
                                name: format!("scene-{}", scene_count + 1),
                                title: format!("场景 {}", scene_count + 1),
                                state: DefinitionState::Known,
                                icon: None,
                                page_id: None,
                                enabled: true,
                                children: Vec::new(),
                                required_permissions: Vec::new(),
                            };
                            submit_patches(
                                scene_api.clone(),
                                scene_application_id.clone(),
                                version,
                                vec![GraphPatch::Insert {
                                    parent_id: program_id,
                                    collection: ChildCollection::Menus,
                                    index: scene_count,
                                    entity: GraphEntity::Menu(scene),
                                }],
                                generation,
                                status,
                            );
                        },
                        "+ 场景"
                    }
                    Button {
                        variant: ButtonVariant::Outline,
                        disabled: first_scene.is_none(),
                        onclick: move |_| {
                            let Some(scene_id) = first_scene else { return; };
                            let page_id = SymbolId::new();
                            let route_id = SymbolId::new();
                            let page_name = format!("page-{}", page_count + 1);
                            let page = PageDefinition {
                                id: page_id,
                                name: page_name.clone(),
                                title: format!("页面 {}", page_count + 1),
                                state: DefinitionState::Known,
                                root: ComponentNode {
                                    id: SymbolId::new(),
                                    component: "ui.section".to_owned(),
                                    state: DefinitionState::Known,
                                    properties: BTreeMap::new(),
                                    content: None,
                                    events: BTreeMap::new(),
                                    children: Vec::new(),
                                    style: ComponentStyle::default(),
                                },
                                page_state: Vec::new(),
                                data_sources: Vec::new(),
                            };
                            let route = RouteDefinition {
                                id: route_id,
                                name: page_name.clone(),
                                path: format!("/{page_name}"),
                                page_id,
                                state: DefinitionState::Known,
                                required_permissions: Vec::new(),
                            };
                            let menu = MenuDefinition {
                                id: SymbolId::new(),
                                name: page_name,
                                title: page.title.clone(),
                                state: DefinitionState::Known,
                                icon: None,
                                page_id: Some(page_id),
                                enabled: true,
                                children: Vec::new(),
                                required_permissions: Vec::new(),
                            };
                            submit_patches(
                                page_api.clone(),
                                page_application_id.clone(),
                                version,
                                vec![
                                    GraphPatch::Insert {
                                        parent_id: program_id,
                                        collection: ChildCollection::Pages,
                                        index: page_count,
                                        entity: GraphEntity::Page(page),
                                    },
                                    GraphPatch::Insert {
                                        parent_id: program_id,
                                        collection: ChildCollection::Routes,
                                        index: page_count,
                                        entity: GraphEntity::Route(route),
                                    },
                                    GraphPatch::Insert {
                                        parent_id: scene_id,
                                        collection: ChildCollection::MenuChildren,
                                        index: 0,
                                        entity: GraphEntity::Menu(menu),
                                    },
                                ],
                                generation,
                                status,
                            );
                        },
                        "+ 页面"
                    }
                }
            }
            div { class: "aio-menu-table-scroll",
                div { class: "aio-menu-table", role: "table", aria_label: "场景与菜单",
                    div { class: "aio-menu-table__header", role: "row",
                        span { role: "columnheader", "菜单名称" }
                        span { role: "columnheader", "图标" }
                        span { role: "columnheader", "排序" }
                        span { role: "columnheader", "权限标识" }
                        span { role: "columnheader", "组件路径" }
                        span { role: "columnheader", "组件名称" }
                        span { role: "columnheader", "状态" }
                        span { role: "columnheader", "操作" }
                    }
                    if draft.definition.menus.is_empty() {
                        div { class: "aio-menu-table__empty", "暂无场景，请先新建场景" }
                    } else {
                        for (index, menu) in draft.definition.menus.iter().cloned().enumerate() {
                            {menu_table_rows(
                                menu,
                                0,
                                index,
                                program_id,
                                ChildCollection::Menus,
                                scene_count,
                                table_context.clone(),
                            )}
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct MenuTableContext {
    api_base_url: String,
    application_id: String,
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
                        drop_context.application_id.clone(),
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
                span { class: "aio-menu-table__component", role: "cell", "{page_name}" }
                label { class: "aio-menu-switch", title: if menu.enabled { "已启用" } else { "已停用" },
                    input {
                        r#type: "checkbox",
                        checked: menu.enabled,
                        aria_label: if menu.enabled { "停用菜单" } else { "启用菜单" },
                        onchange: move |event| submit_patches(
                            enable_context.api_base_url.clone(),
                            enable_context.application_id.clone(),
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
                                add_context.application_id.clone(),
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
                            delete_context.application_id.clone(),
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
                submit_context.application_id.clone(),
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
                label { r#for: "menu-page-{menu_id}", "组件名称" }
                select { id: "menu-page-{menu_id}", name: "page_id", class: "aio-input",
                    option { value: "", selected: menu.page_id.is_none(), "无页面（目录）" }
                    for page in context.pages.iter() {
                        option { value: "{page.id}", selected: menu.page_id == Some(page.id), "{page.name} · {page.title}" }
                    }
                }
            }
            div { class: "aio-menu-table__editor-field aio-menu-table__editor-field--path",
                label { r#for: "menu-path-{menu_id}", "组件路径" }
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

fn canvas_panel(
    draft: &DraftSnapshot,
    selected_page: Signal<Option<SymbolId>>,
    selected_component: Signal<Option<SymbolId>>,
    components: Result<&Arc<ComponentIndex>, &String>,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let selected = selected_page()
        .and_then(|id| draft.definition.pages.iter().find(|page| page.id == id))
        .or_else(|| draft.definition.pages.first());
    let component_index = components.ok();
    let catalog = component_index
        .map(|index| index.browser_catalog())
        .unwrap_or_default();
    let application_id = draft.application_id.clone();
    let version = draft.version;
    let selected_node = selected.and_then(|page| {
        selected_component()
            .and_then(|component_id| find_component(&page.root, component_id))
            .or(Some(&page.root))
    });
    let selected_metadata = selected_node.and_then(|node| {
        component_index.and_then(|index| index.resolve_canonical(&node.component).ok())
    });
    let insertion_parent = selected_node
        .filter(|_| {
            selected_metadata
                .is_some_and(|provider| provider.definition().shape != ComponentShape::Leaf)
        })
        .or_else(|| selected.map(|page| &page.root));
    let drop_parent = insertion_parent.map(|node| node.id);
    let child_index = insertion_parent
        .map(|node| node.children.len())
        .unwrap_or_default();
    let page_states = selected
        .map(|page| page.page_state.clone())
        .unwrap_or_default();
    let data_sources = selected
        .map(|page| page.data_sources.clone())
        .unwrap_or_default();
    rsx! {
        div { class: "aio-studio-canvas-grid grid gap-3 xl:grid-cols-[14rem_minmax(0,1fr)_18rem]",
            aside { class: "rounded-md border p-3",
                strong { class: "text-sm", "页面" }
                div { class: "mt-2 space-y-1", for page in &draft.definition.pages {
                    {page_selector(page.id, page.title.clone(), selected_page)}
                } }
                if let Some(page) = selected.cloned() {
                    {page_contract_editor(
                        page,
                        draft.definition.functions.clone(),
                        api_base_url.clone(),
                        application_id.clone(),
                        version,
                        generation,
                        status,
                    )}
                }
            }
            div {
                class: "aio-studio-canvas min-h-[34rem] rounded-md border bg-muted/20 p-4",
                ondragover: move |event| event.prevent_default(),
                ondrop: move |event| {
                    event.prevent_default();
                    let Some(parent_id) = drop_parent else { return; };
                    let Some(component) = event.data_transfer().get_data("text/plain") else { return; };
                    submit_patches(
                        api_base_url.clone(),
                        application_id.clone(),
                        version,
                        vec![GraphPatch::Insert {
                            parent_id,
                            collection: ChildCollection::ComponentChildren,
                            index: child_index,
                            entity: GraphEntity::Component(ComponentNode {
                                id: SymbolId::new(),
                                component,
                                state: DefinitionState::Known,
                                properties: BTreeMap::new(),
                                content: None,
                                events: BTreeMap::new(),
                                children: Vec::new(),
                                style: ComponentStyle::default(),
                            }),
                        }],
                        generation,
                        status,
                    );
                },
                if let Some(page) = selected {
                    div { class: "rounded-md border border-dashed bg-background p-4",
                        strong { "{page.title}" }
                        if let Some(index) = component_index {
                            div { class: "my-3 rounded border bg-background p-3",
                                {draft_page_preview(page, Arc::clone(index))}
                            }
                        }
                        {component_tree(&page.root, selected_component)}
                    }
                } else {
                    {empty_panel("请先新建页面")}
                }
            }
            aside { class: "aio-studio-component-catalog max-h-[34rem] overflow-auto rounded-md border p-3",
                if let Some(node) = selected_node {
                    {component_inspector(
                        node.clone(),
                        selected_metadata.map(|value| value.properties().clone()).unwrap_or_default(),
                        selected_metadata.map(|value| value.events().keys().cloned().collect()).unwrap_or_default(),
                        draft.definition.functions.clone(),
                        page_states.clone(),
                        data_sources.clone(),
                        api_base_url.clone(),
                        application_id.clone(),
                        version,
                        generation,
                        status,
                    )}
                }
                strong { class: "mt-4 block text-sm", "组件目录" }
                div { class: "mt-2 space-y-2", for (canonical_id, entry) in catalog {
                    {catalog_item(
                        canonical_id,
                        entry,
                        component_index.map(Arc::clone),
                        drop_parent,
                        child_index,
                        api_base_url.clone(),
                        application_id.clone(),
                        version,
                        generation,
                        status,
                    )}
                } }
            }
        }
    }
}

fn draft_page_preview(page: &PageDefinition, components: Arc<ComponentIndex>) -> Element {
    let plan = crate::preview_render_plan(page);
    let data = DynamicRenderData {
        page_state: plan.page_state.clone(),
        data_sources: plan
            .data_sources
            .iter()
            .map(|source| (source.id, serde_json::Value::Null))
            .collect(),
        ..DynamicRenderData::default()
    };
    DynamicRenderer::new(components).render(&plan, &data, Callback::new(|_| {}))
}

fn page_selector(
    page_id: SymbolId,
    title: String,
    mut selected_page: Signal<Option<SymbolId>>,
) -> Element {
    rsx! {
        button {
            class: "w-full rounded px-2 py-1.5 text-left text-sm hover:bg-accent",
            onclick: move |_| selected_page.set(Some(page_id)),
            "{title}"
        }
    }
}

fn page_contract_editor(
    page: PageDefinition,
    functions: Vec<FunctionDefinition>,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let page_id = page.id;
    let state_count = page.page_state.len();
    let source_count = page.data_sources.len();
    let state_api = api_base_url.clone();
    let state_application_id = application_id.clone();
    let has_functions = !functions.is_empty();
    rsx! {
        section { class: "mt-4 space-y-3 border-t pt-3",
            strong { class: "text-xs", "页面状态" }
            for state in &page.page_state {
                div { class: "rounded border px-2 py-1 text-xs",
                    span { class: "font-medium", "{state.name}" }
                    span { class: "ml-1 text-muted-foreground", "{value_type_label(&state.value_type)}" }
                }
            }
            form { class: "space-y-2", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name");
                if name.trim().is_empty() { return; }
                let value_type = value_type_from_key(&form_text(&event, "value_type"));
                let initial_value = parse_initial_value(&value_type, &form_text(&event, "initial_value"));
                submit_patches(
                    state_api.clone(), state_application_id.clone(), version,
                    vec![GraphPatch::Insert {
                        parent_id: page_id,
                        collection: ChildCollection::PageStates,
                        index: state_count,
                        entity: GraphEntity::PageState(PageStateDefinition {
                            id: SymbolId::new(), name, value_type, initial_value,
                        }),
                    }], generation, status,
                );
            },
                input { name: "name", class: "aio-input", placeholder: "状态名称" }
                {value_type_select("value_type")}
                input { name: "initial_value", class: "aio-input", placeholder: "初始值" }
                Button { class: "h-8 w-full text-xs", button_type: "submit", "添加状态" }
            }
            strong { class: "block border-t pt-3 text-xs", "数据源" }
            for source in &page.data_sources {
                div { class: "rounded border px-2 py-1 text-xs", "{source.name}" }
            }
            form { class: "space-y-2", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name");
                let Ok(function_id) = SymbolId::parse(&form_text(&event, "function_id")) else { return; };
                if name.trim().is_empty() { return; }
                submit_patches(
                    api_base_url.clone(), application_id.clone(), version,
                    vec![GraphPatch::Insert {
                        parent_id: page_id,
                        collection: ChildCollection::DataSources,
                        index: source_count,
                        entity: GraphEntity::DataSource(DataSourceDefinition {
                            id: SymbolId::new(), name, function_id, parameters: BTreeMap::new(),
                        }),
                    }], generation, status,
                );
            },
                input { name: "name", class: "aio-input", placeholder: "数据源名称" }
                select { name: "function_id", class: "aio-input",
                    option { value: "", "选择查询逻辑" }
                    for function in functions {
                        option { value: "{function.id}", "{function.title}" }
                    }
                }
                Button { class: "h-8 w-full text-xs", button_type: "submit", disabled: !has_functions, "绑定数据源" }
            }
        }
    }
}

fn component_tree(
    node: &ComponentNode,
    mut selected_component: Signal<Option<SymbolId>>,
) -> Element {
    let node_id = node.id;
    let label = node
        .component
        .rsplit("::")
        .next()
        .unwrap_or(&node.component)
        .to_owned();
    rsx! {
        div {
            class: if selected_component() == Some(node_id) {
                "mt-2 rounded-md border border-primary bg-primary/5 p-2 text-xs"
            } else {
                "mt-2 rounded-md border p-2 text-xs"
            },
            onclick: move |event| {
                event.stop_propagation();
                selected_component.set(Some(node_id));
            },
            code { "{label}" }
            if !node.children.is_empty() {
                div { class: "ml-3 border-l pl-2", for child in &node.children { {component_tree(child, selected_component)} } }
            }
        }
    }
}

fn component_inspector(
    node: ComponentNode,
    properties: BTreeMap<String, ComponentPropertySpec>,
    events: Vec<String>,
    functions: Vec<FunctionDefinition>,
    page_states: Vec<PageStateDefinition>,
    data_sources: Vec<DataSourceDefinition>,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let node_id = node.id;
    let component_name = node
        .component
        .rsplit("::")
        .next()
        .unwrap_or(&node.component)
        .to_owned();
    let content = property_text(node.content.as_ref());
    let style = node.style.clone();
    rsx! {
        section { class: "space-y-3 border-b pb-4",
            div {
                strong { class: "block text-sm", "属性与绑定" }
                code { class: "text-xs text-muted-foreground", "{component_name}" }
            }
            form { class: "space-y-2", onsubmit: move |event| {
                event.prevent_default();
                submit_patches(
                    api_base_url.clone(), application_id.clone(), version,
                    vec![GraphPatch::SetProperty {
                        target_id: node_id,
                        property: crate::EditableProperty::ComponentContent,
                        value: serde_json::to_value(PropertyValue::text(form_text(&event, "content")))
                            .unwrap_or(serde_json::Value::Null),
                    }], generation, status,
                );
            },
                label { class: "block text-xs text-muted-foreground", "内容" }
                input { name: "content", class: "aio-input", value: content }
                Button { class: "h-8 px-3 text-xs", button_type: "submit", "保存" }
            }
            for (name, spec) in properties {
                {component_property_editor(
                    node_id,
                    name.clone(),
                    spec,
                    node.properties.get(&name).cloned(),
                    page_states.clone(),
                    data_sources.clone(),
                    api_base_url.clone(),
                    application_id.clone(),
                    version,
                    generation,
                    status,
                )}
            }
            {component_style_editor(
                node_id,
                style,
                api_base_url.clone(),
                application_id.clone(),
                version,
                generation,
                status,
            )}
            for event_name in events {
                {component_event_editor(
                    node_id,
                    event_name.clone(),
                    node.events.get(&event_name).copied(),
                    functions.clone(),
                    api_base_url.clone(),
                    application_id.clone(),
                    version,
                    generation,
                    status,
                )}
            }
        }
    }
}

fn component_style_editor(
    node_id: SymbolId,
    current: ComponentStyle,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let variant = current.variant.clone().unwrap_or_default();
    rsx! {
        form { class: "space-y-2 border-t pt-3", onsubmit: move |event| {
            event.prevent_default();
            let variant = form_text(&event, "variant");
            let columns = form_text(&event, "columns")
                .parse::<u8>()
                .ok()
                .filter(|value| (1..=12).contains(value));
            let style = ComponentStyle {
                variant: (!variant.trim().is_empty()).then_some(variant),
                spacing: spacing_token_from_key(&form_text(&event, "spacing")),
                columns,
                alignment: alignment_from_key(&form_text(&event, "alignment")),
                width: current.width.clone(),
                height: current.height.clone(),
                responsive: current.responsive.clone(),
            };
            submit_patches(
                api_base_url.clone(), application_id.clone(), version,
                vec![GraphPatch::SetProperty {
                    target_id: node_id,
                    property: crate::EditableProperty::ComponentStyle,
                    value: serde_json::to_value(style).unwrap_or(serde_json::Value::Null),
                }], generation, status,
            );
        },
            label { class: "block text-xs font-medium", "布局令牌" }
            input { name: "variant", class: "aio-input", value: variant, placeholder: "组件 variant" }
            select { name: "spacing", class: "aio-input",
                option { value: "", "默认间距" }
                option { value: "none", "无间距" }
                option { value: "xs", "极小" }
                option { value: "sm", "小" }
                option { value: "md", "中" }
                option { value: "lg", "大" }
                option { value: "xl", "极大" }
            }
            input { name: "columns", class: "aio-input", r#type: "number", min: "1", max: "12", placeholder: "栅格列数" }
            select { name: "alignment", class: "aio-input",
                option { value: "", "默认对齐" }
                option { value: "start", "起始" }
                option { value: "center", "居中" }
                option { value: "end", "末端" }
                option { value: "stretch", "拉伸" }
                option { value: "space_between", "两端" }
            }
            Button { class: "h-8 px-3 text-xs", button_type: "submit", "保存布局" }
        }
    }
}

fn spacing_token_from_key(key: &str) -> Option<SpacingToken> {
    match key {
        "none" => Some(SpacingToken::None),
        "xs" => Some(SpacingToken::Xs),
        "sm" => Some(SpacingToken::Sm),
        "md" => Some(SpacingToken::Md),
        "lg" => Some(SpacingToken::Lg),
        "xl" => Some(SpacingToken::Xl),
        _ => None,
    }
}

fn alignment_from_key(key: &str) -> Option<Alignment> {
    match key {
        "start" => Some(Alignment::Start),
        "center" => Some(Alignment::Center),
        "end" => Some(Alignment::End),
        "stretch" => Some(Alignment::Stretch),
        "space_between" => Some(Alignment::SpaceBetween),
        _ => None,
    }
}

fn component_property_editor(
    node_id: SymbolId,
    name: String,
    spec: ComponentPropertySpec,
    current: Option<PropertyValue>,
    page_states: Vec<PageStateDefinition>,
    data_sources: Vec<DataSourceDefinition>,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let current = property_text(current.as_ref());
    let label = name.clone();
    let binding_name = name.clone();
    let binding_api = api_base_url.clone();
    let binding_application_id = application_id.clone();
    let has_bindings = !page_states.is_empty() || !data_sources.is_empty();
    rsx! {
        form { class: "space-y-2", onsubmit: move |event| {
            event.prevent_default();
            let raw = form_text(&event, "value");
            let Ok(value) = parse_component_value(spec.kind, &raw) else { return; };
            submit_patches(
                api_base_url.clone(), application_id.clone(), version,
                vec![GraphPatch::SetProperty {
                    target_id: node_id,
                    property: crate::EditableProperty::ComponentProperty(name.clone()),
                    value: serde_json::to_value(PropertyValue::Literal { value })
                        .unwrap_or(serde_json::Value::Null),
                }], generation, status,
            );
        },
            label { class: "block text-xs text-muted-foreground", "{label}" }
            if spec.choices.is_empty() {
                input { name: "value", class: "aio-input", value: current }
            } else {
                select { name: "value", class: "aio-input", for choice in spec.choices {
                    option { value: "{choice}", selected: choice == current, "{choice}" }
                } }
            }
            Button { class: "h-8 px-3 text-xs", button_type: "submit", "保存" }
        }
        if has_bindings {
            form { class: "space-y-2", onsubmit: move |event| {
                event.prevent_default();
                let binding = form_text(&event, "binding");
                let property_value = if let Some(value) = binding.strip_prefix("state:") {
                    SymbolId::parse(value).ok().map(|state_id| PropertyValue::PageState { state_id })
                } else if let Some(value) = binding.strip_prefix("source:") {
                    SymbolId::parse(value).ok().map(|source_id| PropertyValue::DataSource {
                        source_id,
                        path: Vec::new(),
                    })
                } else {
                    None
                };
                let Some(property_value) = property_value else { return; };
                submit_patches(
                    binding_api.clone(), binding_application_id.clone(), version,
                    vec![GraphPatch::SetProperty {
                        target_id: node_id,
                        property: crate::EditableProperty::ComponentProperty(binding_name.clone()),
                        value: serde_json::to_value(property_value).unwrap_or(serde_json::Value::Null),
                    }], generation, status,
                );
            },
                select { name: "binding", class: "aio-input",
                    for state in page_states {
                        option { value: "state:{state.id}", "状态 · {state.name}" }
                    }
                    for source in data_sources {
                        option { value: "source:{source.id}", "数据源 · {source.name}" }
                    }
                }
                Button { class: "h-8 px-3 text-xs", button_type: "submit", "绑定 {label}" }
            }
        }
    }
}

fn component_event_editor(
    node_id: SymbolId,
    event_name: String,
    selected: Option<SymbolId>,
    functions: Vec<FunctionDefinition>,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let label = event_name.clone();
    rsx! {
        form { class: "space-y-2 border-t pt-3", onsubmit: move |event| {
            event.prevent_default();
            let function_id = form_text(&event, "function_id");
            let value = if function_id.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(function_id)
            };
            submit_patches(
                api_base_url.clone(), application_id.clone(), version,
                vec![GraphPatch::SetProperty {
                    target_id: node_id,
                    property: crate::EditableProperty::ComponentEvent(event_name.clone()),
                    value,
                }], generation, status,
            );
        },
            label { class: "block text-xs font-medium", "事件 · {label}" }
            select { name: "function_id", class: "aio-input",
                option { value: "", "未绑定" }
                for function in functions {
                    option { value: "{function.id}", selected: selected == Some(function.id), "{function.title}" }
                }
            }
            Button { class: "h-8 px-3 text-xs", button_type: "submit", "绑定" }
        }
    }
}

fn property_text(value: Option<&PropertyValue>) -> String {
    match value {
        Some(PropertyValue::Literal { value }) => value
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string()),
        _ => String::new(),
    }
}

fn parse_component_value(
    kind: ComponentPropertyKind,
    value: &str,
) -> Result<serde_json::Value, String> {
    match kind {
        ComponentPropertyKind::Boolean => value
            .parse::<bool>()
            .map(serde_json::Value::Bool)
            .map_err(|error| error.to_string()),
        ComponentPropertyKind::Number => value
            .parse::<f64>()
            .map_err(|error| error.to_string())
            .and_then(|number| {
                serde_json::Number::from_f64(number)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| "数字无效".to_owned())
            }),
        ComponentPropertyKind::Json => {
            serde_json::from_str(value).map_err(|error| error.to_string())
        }
        ComponentPropertyKind::Text
        | ComponentPropertyKind::Choice
        | ComponentPropertyKind::Action => Ok(serde_json::Value::String(value.to_owned())),
    }
}

fn component_contains_id(node: &ComponentNode, target: Option<SymbolId>) -> bool {
    target.is_some_and(|target| {
        node.id == target
            || node
                .children
                .iter()
                .any(|child| component_contains_id(child, Some(target)))
    })
}

fn find_component(node: &ComponentNode, target: SymbolId) -> Option<&ComponentNode> {
    if node.id == target {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_component(child, target))
}

fn catalog_item(
    canonical_id: String,
    entry: ComponentCatalogEntry,
    components: Option<Arc<ComponentIndex>>,
    parent_id: Option<SymbolId>,
    index: usize,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let label = components
        .as_ref()
        .and_then(|index| index.resolve_canonical(&canonical_id).ok())
        .and_then(|component| component.semantic_names().first().cloned())
        .unwrap_or_else(|| component_label(&canonical_id));
    let shape = component_shape_label(entry.shape);
    let drag_component = canonical_id.clone();
    let button_label = label.clone();
    rsx! {
        div {
            class: "cursor-grab rounded-md border bg-background p-2 text-xs active:cursor-grabbing",
            draggable: "true",
            ondragstart: move |event| {
                let _ = event.data_transfer().set_data("text/plain", &drag_component);
            },
            div { class: "flex items-center justify-between gap-2",
                div { class: "min-w-0",
                    p { class: "truncate font-medium", "{label}" }
                    p { class: "text-muted-foreground", "{shape}" }
                }
                button {
                    r#type: "button",
                    class: "aio-studio-catalog-add shrink-0 rounded border text-base hover:bg-accent",
                    title: "添加 {button_label}",
                    aria_label: "添加 {button_label}",
                    disabled: parent_id.is_none(),
                    onclick: move |event| {
                        event.stop_propagation();
                        let Some(parent_id) = parent_id else { return; };
                        submit_patches(
                            api_base_url.clone(),
                            application_id.clone(),
                            version,
                            vec![GraphPatch::Insert {
                                parent_id,
                                collection: ChildCollection::ComponentChildren,
                                index,
                                entity: GraphEntity::Component(ComponentNode {
                                    id: SymbolId::new(),
                                    component: canonical_id.clone(),
                                    state: DefinitionState::Known,
                                    properties: BTreeMap::new(),
                                    content: None,
                                    events: BTreeMap::new(),
                                    children: Vec::new(),
                                    style: ComponentStyle::default(),
                                }),
                            }],
                            generation,
                            status,
                        );
                    },
                    "+"
                }
            }
            div { class: "aio-studio-catalog-preview mt-2 min-h-16 overflow-hidden rounded border bg-muted/20 p-2 pointer-events-none",
                if let Some(components) = components {
                    {render_catalog_preview(components, &canonical_id, &entry, &label)}
                }
            }
        }
    }
}

fn render_catalog_preview(
    components: Arc<ComponentIndex>,
    canonical_id: &str,
    entry: &ComponentCatalogEntry,
    label: &str,
) -> Element {
    let Ok(component) = components.resolve_canonical(canonical_id) else {
        return rsx! {};
    };
    component.render(ComponentRenderContext {
        component_id: SymbolId::new(),
        properties: catalog_preview_properties(entry, label),
        content: Some(label.to_owned()),
        children: catalog_preview_children(&entry.spec.html_tag),
        dispatch: Callback::new(|_| {}),
        style: ComponentStyle::default(),
    })
}

fn catalog_preview_properties(
    entry: &ComponentCatalogEntry,
    label: &str,
) -> BTreeMap<String, serde_json::Value> {
    entry
        .properties
        .iter()
        .map(|(name, spec)| {
            let value = match spec.kind {
                ComponentPropertyKind::Boolean => serde_json::Value::Bool(name != "disabled"),
                ComponentPropertyKind::Number => serde_json::json!(68),
                ComponentPropertyKind::Json => serde_json::json!([]),
                ComponentPropertyKind::Choice => spec
                    .choices
                    .first()
                    .cloned()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null),
                ComponentPropertyKind::Text | ComponentPropertyKind::Action => {
                    serde_json::Value::String(catalog_preview_text(name, label))
                }
            };
            (name.clone(), value)
        })
        .collect()
}

fn catalog_preview_text(name: &str, label: &str) -> String {
    match name {
        "label" | "title" | "text" | "tx" => label.to_owned(),
        "message" => "示例内容".to_owned(),
        "placeholder" | "ph" => "请输入".to_owned(),
        "status" => "进行中".to_owned(),
        "options" => "选项一,选项二".to_owned(),
        "href" => "#".to_owned(),
        "type" => "text".to_owned(),
        "variant" | "v" => "default".to_owned(),
        _ => String::new(),
    }
}

fn catalog_preview_children(html_tag: &str) -> Element {
    match html_tag {
        "table" => rsx! { tbody { tr { td { class: "p-2", "表格内容" } } } },
        "thead" => rsx! { tr { th { class: "p-2 text-left", "表头" } } },
        "tbody" => rsx! { tr { td { class: "p-2", "表格内容" } } },
        "tr" => rsx! { td { class: "p-2", "单元格" } },
        "ul" | "ol" => rsx! { li { "列表项" } },
        _ => rsx! { span { class: "text-xs text-muted-foreground", "内容" } },
    }
}

fn component_label(canonical_id: &str) -> String {
    canonical_id
        .rsplit('.')
        .next()
        .unwrap_or(canonical_id)
        .replace('-', " ")
}

const fn component_shape_label(shape: ComponentShape) -> &'static str {
    match shape {
        ComponentShape::Leaf => "叶子组件",
        ComponentShape::Container => "容器组件",
        ComponentShape::Dual => "内容/容器组件",
    }
}

fn logic_panel(
    draft: &DraftSnapshot,
    mut selected_function: Signal<Option<SymbolId>>,
    capabilities: crate::CapabilityCatalog,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let application_id = draft.application_id.clone();
    let program_id = draft.definition.id;
    let version = draft.version;
    let count = draft.definition.functions.len();
    let permission_count = draft.definition.permissions.len();
    let functions = draft.definition.functions.clone();
    let selected = selected_function()
        .and_then(|id| functions.iter().find(|value| value.id == id).cloned())
        .or_else(|| functions.first().cloned());
    let selected_id = selected.as_ref().map(|value| value.id);
    let node_count = selected
        .as_ref()
        .map(|value| value.graph.nodes.len())
        .unwrap_or_default();
    let create_api = api_base_url.clone();
    let create_application_id = application_id.clone();
    let first_state = draft
        .definition
        .pages
        .iter()
        .flat_map(|page| &page.page_state)
        .next()
        .map(|state| state.id);
    let first_source = draft
        .definition
        .pages
        .iter()
        .flat_map(|page| &page.data_sources)
        .next()
        .map(|source| source.id);
    let first_component = draft.definition.pages.first().map(|page| page.root.id);
    rsx! {
        div { class: "aio-studio-logic-grid grid gap-3 xl:grid-cols-[11rem_minmax(28rem,1fr)_14rem]",
            aside { class: "rounded-md border p-3",
                div { class: "flex items-center justify-between gap-2",
                    strong { class: "text-sm", "逻辑" }
                    Button {
                        size: ButtonSize::IconSm,
                        title: "新建逻辑",
                        aria_label: "新建逻辑",
                        onclick: move |_| {
                        let function_id = SymbolId::new();
                        let permission_id = SymbolId::new();
                        selected_function.set(Some(function_id));
                        submit_patches(
                            create_api.clone(), create_application_id.clone(), version,
                            vec![
                                GraphPatch::Insert {
                                    parent_id: program_id,
                                    collection: ChildCollection::Permissions,
                                    index: permission_count,
                                    entity: GraphEntity::Permission(PermissionDefinition {
                                        id: permission_id,
                                        name: format!("function_{}_effects", count + 1),
                                        title: format!("逻辑 {} Effect", count + 1),
                                        allowed_effects: vec![
                                            EffectKind::UserPrompt,
                                            EffectKind::DatabaseRead,
                                            EffectKind::DatabaseWrite,
                                            EffectKind::Secret,
                                            EffectKind::Capability,
                                        ],
                                    }),
                                },
                                GraphPatch::Insert {
                                    parent_id: program_id,
                                    collection: ChildCollection::Functions,
                                    index: count,
                                    entity: GraphEntity::Function(FunctionDefinition {
                                        id: function_id,
                                        name: format!("function_{}", count + 1),
                                        title: format!("逻辑 {}", count + 1),
                                        state: DefinitionState::Known,
                                        inputs: Vec::new(),
                                        outputs: Vec::new(),
                                        graph: FunctionGraph::default(),
                                        required_permissions: vec![permission_id],
                                    }),
                                }
                            ], generation, status,
                        );
                        },
                        "+"
                    }
                }
                div { class: "mt-2 space-y-1", for function in &functions {
                    {function_selector(function.clone(), selected_id, selected_function)}
                } }
            }
            div { class: "min-w-0",
                if let Some(function) = selected.clone() {
                    LogicWorkflowEditor {
                        key: "{application_id}:{version}:{function.id}",
                        function,
                        api_base_url: api_base_url.clone(),
                        application_id: application_id.clone(),
                        version,
                        generation,
                        status,
                    }
                } else {
                    {empty_panel("新建逻辑后添加节点")}
                }
            }
            aside { class: "space-y-4 rounded-md border p-3",
                strong { class: "text-sm", "节点目录" }
                if let Some(function_id) = selected_id {
                    div { class: "grid grid-cols-2 gap-2",
                        {node_palette_button("常量", FunctionNodeKind::Constant { value: serde_json::Value::String("值".to_owned()), value_type: ValueType::Text }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("对象", FunctionNodeKind::Object { fields: BTreeMap::new() }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("列表", FunctionNodeKind::List { items: Vec::new() }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("格式化", FunctionNodeKind::Format { template: "{0}".to_owned(), values: Vec::new() }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("加法", FunctionNodeKind::Math { operator: MathOperator::Add }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("相等", FunctionNodeKind::Compare { operator: CompareOperator::Equal }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("且", FunctionNodeKind::Boolean { operator: BooleanOperator::And }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("条件", FunctionNodeKind::Condition, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("校验", FunctionNodeKind::ValidateForm { rules: Vec::new() }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("确认", FunctionNodeKind::Confirm { message: PropertyValue::text("确认执行？") }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("通知", FunctionNodeKind::Notify { level: NotificationLevel::Success }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("失败", FunctionNodeKind::Fail { code: "BUSINESS_REJECTED".to_owned() }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("返回", FunctionNodeKind::Return, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(port) = selected.as_ref().and_then(|function| function.inputs.first()) {
                        {node_palette_button("输入", FunctionNodeKind::Input { port_id: port.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(port) = selected.as_ref().and_then(|function| function.outputs.first()) {
                        {node_palette_button("输出", FunctionNodeKind::Output { port_id: port.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(state_id) = first_state {
                        {node_palette_button("改状态", FunctionNodeKind::SetState { state_id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(source_id) = first_source {
                        {node_palette_button("刷新", FunctionNodeKind::Refresh { source_id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(component_id) = first_component {
                        {node_palette_button("开弹窗", FunctionNodeKind::OpenDialog { component_id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("关弹窗", FunctionNodeKind::CloseDialog { component_id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(route) = draft.definition.routes.first() {
                        {node_palette_button("导航", FunctionNodeKind::Navigate { route_id: route.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    if let Some(model) = draft.definition.models.first() {
                        {node_palette_button("查询", FunctionNodeKind::QueryRecords { model_id: model.id, limit: 50 }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("新增", FunctionNodeKind::CreateRecord { model_id: model.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("读取", FunctionNodeKind::ReadRecord { model_id: model.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("更新", FunctionNodeKind::UpdateRecord { model_id: model.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                        {node_palette_button("删除", FunctionNodeKind::DeleteRecord { model_id: model.id }, function_id, node_count, api_base_url.clone(), application_id.clone(), version, generation, status)}
                    }
                    for (capability_id, capability) in capabilities.capabilities {
                        for operation in capability.operations.keys() {
                            {node_palette_button(
                                "Capability",
                                FunctionNodeKind::Capability {
                                    capability_id: capability_id.clone(),
                                    operation: operation.clone(),
                                },
                                function_id,
                                node_count,
                                api_base_url.clone(),
                                application_id.clone(),
                                version,
                                generation,
                                status,
                            )}
                        }
                    }
                    if let Some(function) = selected.clone() {
                        {function_contract_editor(
                            function,
                            api_base_url.clone(),
                            application_id.clone(),
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

fn function_selector(
    function: FunctionDefinition,
    selected_id: Option<SymbolId>,
    mut selected_function: Signal<Option<SymbolId>>,
) -> Element {
    let function_id = function.id;
    let class = if Some(function_id) == selected_id {
        "w-full rounded-md bg-primary px-2 py-2 text-left text-xs text-primary-foreground"
    } else {
        "w-full rounded-md px-2 py-2 text-left text-xs hover:bg-accent"
    };
    rsx! {
        button { class, onclick: move |_| selected_function.set(Some(function_id)),
            span { class: "block font-medium", "{function.title}" }
            span { class: "opacity-70", "{function.graph.nodes.len()} 节点 · {function.graph.edges.len()} 连线" }
        }
    }
}

fn node_palette_button(
    label: &'static str,
    kind: FunctionNodeKind,
    function_id: SymbolId,
    index: usize,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    rsx! {
        Button {
            class: "w-full justify-start",
            variant: ButtonVariant::Outline,
            size: ButtonSize::Sm,
            onclick: move |_| {
            let node = FunctionNode {
                id: SymbolId::new(),
                name: format!("{}-{}", node_kind_name(&kind), index + 1),
                state: DefinitionState::Known,
                editor: FunctionNodeEditor {
                    x: 24 + ((index % 3) as i32 * 216),
                    y: 72 + ((index / 3) as i32 * 120),
                },
                kind: kind.clone(),
            };
            submit_patches(
                api_base_url.clone(), application_id.clone(), version,
                vec![GraphPatch::Insert {
                    parent_id: function_id,
                    collection: ChildCollection::FunctionNodes,
                    index,
                    entity: GraphEntity::FunctionNode(node),
                }], generation, status,
            );
            },
            "{label}"
        }
    }
}

#[component]
fn LogicWorkflowEditor(
    function: FunctionDefinition,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let function_id = function.id;
    let original_graph = function.graph.clone();
    let workflow_nodes = function
        .graph
        .nodes
        .iter()
        .map(logic_workflow_node)
        .collect();
    let workflow_edges = function
        .graph
        .edges
        .iter()
        .map(|edge| WorkflowEdge {
            from: edge.from_node.to_string(),
            to: edge.to_node.to_string(),
            style: WorkflowEdgeStyle::Solid,
            label: None,
        })
        .collect();
    let workflow = use_workflow(workflow_nodes, workflow_edges);
    let mut persistence_generation = use_signal(|| 0_u64);

    use_effect(move || {
        let local_nodes = workflow.nodes.read().clone();
        let local_positions = workflow.positions.read().clone();
        let local_edges = workflow.edges.read().clone();
        let known_ids = original_graph
            .nodes
            .iter()
            .map(|node| node.id.to_string())
            .collect::<BTreeSet<_>>();
        let retained_indices = local_nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| known_ids.contains(&node.id).then_some(index))
            .collect::<Vec<_>>();

        if retained_indices.len() != local_nodes.len() {
            let retained_nodes = retained_indices
                .iter()
                .map(|index| local_nodes[*index].clone())
                .collect::<Vec<_>>();
            let retained_positions = retained_indices
                .iter()
                .filter_map(|index| local_positions.get(*index).copied())
                .collect::<Vec<_>>();
            let retained_ids = retained_nodes
                .iter()
                .map(|node| node.id.clone())
                .collect::<BTreeSet<_>>();
            let retained_edges = local_edges
                .into_iter()
                .filter(|edge| retained_ids.contains(&edge.from) && retained_ids.contains(&edge.to))
                .collect::<Vec<_>>();
            let mut state = workflow;
            state.positions.set(retained_positions);
            state.nodes.set(retained_nodes);
            state.edges.set(retained_edges);
            return;
        }

        let patches = logic_workflow_patches(
            function_id,
            &original_graph,
            &local_nodes,
            &local_positions,
            &local_edges,
        );
        let next_generation = *persistence_generation.peek() + 1;
        persistence_generation.set(next_generation);
        if patches.is_empty() {
            return;
        }
        let patch_api_base_url = api_base_url.clone();
        let patch_application_id = application_id.clone();
        spawn(async move {
            TimeoutFuture::new(300).await;
            if *persistence_generation.peek() != next_generation {
                return;
            }
            submit_patches(
                patch_api_base_url,
                patch_application_id,
                version,
                patches,
                generation,
                status,
            );
        });
    });

    let zoom_percent = (workflow.zoom_value() * 100.0).round() as i32;
    rsx! {
        section { class: "space-y-3",
            div { class: "flex min-h-10 items-center justify-between gap-3 rounded-md border bg-card px-3",
                div { class: "min-w-0",
                    strong { class: "block truncate text-sm", "{function.title}" }
                    span { class: "text-xs text-muted-foreground", "{function.graph.nodes.len()} 节点 · {function.graph.edges.len()} 连线" }
                }
                Badge { variant: BadgeVariant::Muted, "已同步" }
            }
            WorkflowCanvas {
                state: workflow,
                overlay: rsx! {
                    div {
                        class: "flex items-center gap-1 rounded-md border bg-background/90 p-1 shadow-sm backdrop-blur-sm",
                        style: "position:absolute;top:12px;right:12px;",
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::IconXs,
                            title: "缩小",
                            aria_label: "缩小",
                            onclick: move |_| {
                                let mut state = workflow;
                                state.zoom_step(1.0 / 1.2);
                            },
                            "−"
                        }
                        span { class: "w-10 text-center text-[11px] tabular-nums text-muted-foreground", "{zoom_percent}%" }
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::IconXs,
                            title: "放大",
                            aria_label: "放大",
                            onclick: move |_| {
                                let mut state = workflow;
                                state.zoom_step(1.2);
                            },
                            "+"
                        }
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Sm,
                            onclick: move |_| {
                                let mut state = workflow;
                                state.fit_to_view(800.0, 450.0, NODE_H);
                            },
                            "适应"
                        }
                    }
                    WorkflowMinimap { state: workflow }
                },
                for (index, node) in workflow.nodes.read().clone().into_iter().enumerate() {
                    WorkflowNodeWrapper { state: workflow, idx: index,
                        WorkflowDefaultNode { node }
                    }
                }
            }
        }
    }
}

fn logic_workflow_node(node: &FunctionNode) -> WorkflowNode {
    WorkflowNode {
        id: node.id.to_string(),
        initial_x: f64::from(node.editor.x),
        initial_y: f64::from(node.editor.y),
        width: 196.0,
        has_target: !matches!(
            node.kind,
            FunctionNodeKind::Constant { .. } | FunctionNodeKind::Input { .. }
        ),
        has_source: !matches!(
            node.kind,
            FunctionNodeKind::Output { .. }
                | FunctionNodeKind::Navigate { .. }
                | FunctionNodeKind::Notify { .. }
                | FunctionNodeKind::Return
                | FunctionNodeKind::Fail { .. }
        ),
        label: node.name.clone(),
        description: node_kind_label(&node.kind).to_owned(),
        kind: logic_workflow_node_kind(&node.kind),
    }
}

fn logic_workflow_node_kind(kind: &FunctionNodeKind) -> WorkflowNodeKind {
    match kind {
        FunctionNodeKind::Input { .. } => WorkflowNodeKind::Trigger,
        FunctionNodeKind::Capability { .. } => WorkflowNodeKind::Agent,
        FunctionNodeKind::Output { .. }
        | FunctionNodeKind::Navigate { .. }
        | FunctionNodeKind::Notify { .. }
        | FunctionNodeKind::Return
        | FunctionNodeKind::Fail { .. } => WorkflowNodeKind::Output,
        _ => WorkflowNodeKind::Data,
    }
}

fn logic_workflow_patches(
    function_id: SymbolId,
    original: &FunctionGraph,
    local_nodes: &[WorkflowNode],
    local_positions: &[(f64, f64)],
    local_edges: &[WorkflowEdge],
) -> Vec<GraphPatch> {
    let mut patches = Vec::new();
    let mut deleted_nodes = BTreeSet::new();

    for node in &original.nodes {
        let Some((index, local_node)) = local_nodes
            .iter()
            .enumerate()
            .find(|(_, local_node)| local_node.id == node.id.to_string())
        else {
            deleted_nodes.insert(node.id);
            continue;
        };
        if let Some((x, y)) = local_positions.get(index) {
            let x = x.round() as i32;
            let y = y.round() as i32;
            if node.editor.x != x || node.editor.y != y {
                patches.push(GraphPatch::SetProperty {
                    target_id: node.id,
                    property: crate::EditableProperty::FunctionNodePosition,
                    value: serde_json::json!({ "x": x, "y": y }),
                });
            }
        }
        let name = local_node.label.trim();
        if !name.is_empty() && name != node.name {
            patches.push(GraphPatch::Rename {
                target_id: node.id,
                name: name.to_owned(),
                title: None,
            });
        }
    }

    for edge in &original.edges {
        let remains = local_edges.iter().any(|local_edge| {
            local_edge.from == edge.from_node.to_string()
                && local_edge.to == edge.to_node.to_string()
        });
        if !remains
            && !deleted_nodes.contains(&edge.from_node)
            && !deleted_nodes.contains(&edge.to_node)
        {
            patches.push(GraphPatch::Disconnect {
                function_id,
                edge_id: edge.id,
            });
        }
    }

    for edge in local_edges {
        let Ok(from_node) = SymbolId::parse(&edge.from) else {
            continue;
        };
        let Ok(to_node) = SymbolId::parse(&edge.to) else {
            continue;
        };
        let exists = original
            .edges
            .iter()
            .any(|item| item.from_node == from_node && item.to_node == to_node);
        if !exists {
            patches.push(GraphPatch::Connect {
                function_id,
                edge: GraphEdge {
                    id: SymbolId::new(),
                    from_node,
                    from_port: "value".to_owned(),
                    to_node,
                    to_port: "value".to_owned(),
                },
            });
        }
    }

    patches.extend(
        deleted_nodes
            .into_iter()
            .map(|target_id| GraphPatch::Delete { target_id }),
    );
    patches
}

fn function_contract_editor(
    function: FunctionDefinition,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let function_id = function.id;
    let input_count = function.inputs.len();
    let output_count = function.outputs.len();
    let input_api = api_base_url.clone();
    let input_application_id = application_id.clone();
    rsx! {
        section { class: "space-y-2 border-t pt-3",
            strong { class: "text-xs", "输入端口" }
            for port in &function.inputs {
                p { class: "rounded border px-2 py-1 text-xs", "{port.name} · {value_type_label(&port.value_type)}" }
            }
            form { class: "space-y-2", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name");
                if name.trim().is_empty() { return; }
                submit_patches(
                    input_api.clone(), input_application_id.clone(), version,
                    vec![GraphPatch::Insert {
                        parent_id: function_id,
                        collection: ChildCollection::FunctionInputs,
                        index: input_count,
                        entity: GraphEntity::Port(PortDefinition {
                            id: SymbolId::new(),
                            name,
                            value_type: value_type_from_key(&form_text(&event, "value_type")),
                        }),
                    }], generation, status,
                );
            },
                input { name: "name", class: "aio-input", placeholder: "输入名称" }
                {value_type_select("value_type")}
                Button { class: "h-8 w-full text-xs", button_type: "submit", "添加输入" }
            }
            strong { class: "block border-t pt-3 text-xs", "输出端口" }
            for port in &function.outputs {
                p { class: "rounded border px-2 py-1 text-xs", "{port.name} · {value_type_label(&port.value_type)}" }
            }
            form { class: "space-y-2", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name");
                if name.trim().is_empty() { return; }
                submit_patches(
                    api_base_url.clone(), application_id.clone(), version,
                    vec![GraphPatch::Insert {
                        parent_id: function_id,
                        collection: ChildCollection::FunctionOutputs,
                        index: output_count,
                        entity: GraphEntity::Port(PortDefinition {
                            id: SymbolId::new(),
                            name,
                            value_type: value_type_from_key(&form_text(&event, "value_type")),
                        }),
                    }], generation, status,
                );
            },
                input { name: "name", class: "aio-input", placeholder: "输出名称" }
                {value_type_select("value_type")}
                Button { class: "h-8 w-full text-xs", button_type: "submit", "添加输出" }
            }
        }
    }
}

fn node_kind_name(kind: &FunctionNodeKind) -> &'static str {
    match kind {
        FunctionNodeKind::Constant { .. } => "constant",
        FunctionNodeKind::Input { .. } => "input",
        FunctionNodeKind::Output { .. } => "output",
        FunctionNodeKind::Object { .. } => "object",
        FunctionNodeKind::List { .. } => "list",
        FunctionNodeKind::FieldAccess { .. } => "field",
        FunctionNodeKind::Format { .. } => "format",
        FunctionNodeKind::Compare { .. } => "compare",
        FunctionNodeKind::Boolean { .. } => "boolean",
        FunctionNodeKind::Math { .. } => "math",
        FunctionNodeKind::Condition => "condition",
        FunctionNodeKind::ForEach { .. } => "foreach",
        FunctionNodeKind::SetState { .. } => "state",
        FunctionNodeKind::ValidateForm { .. } => "validate",
        FunctionNodeKind::CreateRecord { .. } => "create",
        FunctionNodeKind::ReadRecord { .. } => "read",
        FunctionNodeKind::UpdateRecord { .. } => "update",
        FunctionNodeKind::DeleteRecord { .. } => "delete",
        FunctionNodeKind::QueryRecords { .. } => "query",
        FunctionNodeKind::Navigate { .. } => "navigate",
        FunctionNodeKind::Confirm { .. } => "confirm",
        FunctionNodeKind::OpenDialog { .. } => "open-dialog",
        FunctionNodeKind::CloseDialog { .. } => "close-dialog",
        FunctionNodeKind::Notify { .. } => "notify",
        FunctionNodeKind::Refresh { .. } => "refresh",
        FunctionNodeKind::Return => "return",
        FunctionNodeKind::Fail { .. } => "fail",
        FunctionNodeKind::Capability { .. } => "capability",
    }
}

fn node_kind_label(kind: &FunctionNodeKind) -> &'static str {
    match kind {
        FunctionNodeKind::Constant { .. } => "常量",
        FunctionNodeKind::Input { .. } => "输入",
        FunctionNodeKind::Output { .. } => "输出",
        FunctionNodeKind::Object { .. } => "对象",
        FunctionNodeKind::List { .. } => "列表",
        FunctionNodeKind::FieldAccess { .. } => "字段读取",
        FunctionNodeKind::Format { .. } => "格式化",
        FunctionNodeKind::Compare { .. } => "比较",
        FunctionNodeKind::Boolean { .. } => "布尔运算",
        FunctionNodeKind::Math { .. } => "数学运算",
        FunctionNodeKind::Condition => "条件分支",
        FunctionNodeKind::ForEach { .. } => "受控遍历",
        FunctionNodeKind::SetState { .. } => "修改状态",
        FunctionNodeKind::ValidateForm { .. } => "表单校验",
        FunctionNodeKind::CreateRecord { .. } => "新增记录",
        FunctionNodeKind::ReadRecord { .. } => "读取记录",
        FunctionNodeKind::UpdateRecord { .. } => "更新记录",
        FunctionNodeKind::DeleteRecord { .. } => "删除记录",
        FunctionNodeKind::QueryRecords { .. } => "查询记录",
        FunctionNodeKind::Navigate { .. } => "页面导航",
        FunctionNodeKind::Confirm { .. } => "确认操作",
        FunctionNodeKind::OpenDialog { .. } => "打开弹窗",
        FunctionNodeKind::CloseDialog { .. } => "关闭弹窗",
        FunctionNodeKind::Notify { .. } => "发送通知",
        FunctionNodeKind::Refresh { .. } => "刷新数据",
        FunctionNodeKind::Return => "返回结果",
        FunctionNodeKind::Fail { .. } => "失败终止",
        FunctionNodeKind::Capability { .. } => "能力调用",
    }
}

fn models_panel(
    draft: &DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let application_id = draft.application_id.clone();
    let program_id = draft.definition.id;
    let version = draft.version;
    let count = draft.definition.models.len();
    let create_api = api_base_url.clone();
    let create_application_id = application_id.clone();
    rsx! {
        Card {
            CardHeader { CardTitle { "模型设计器" } }
            CardContent { class: "space-y-3",
                Button { onclick: move |_| {
                    let model_id = SymbolId::new();
                    let suffix = model_id.to_string().replace('-', "");
                    submit_patches(
                        create_api.clone(), create_application_id.clone(), version,
                        vec![GraphPatch::Insert {
                            parent_id: program_id,
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
                }, "新建模型" }
                for model in &draft.definition.models {
                    {model_editor(
                        model.clone(),
                        api_base_url.clone(),
                        application_id.clone(),
                        version,
                        generation,
                        status,
                    )}
                }
            }
        }
    }
}

fn model_editor(
    model: ModelDefinition,
    api_base_url: String,
    application_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let model_id = model.id;
    let field_count = model.fields.len();
    let index_count = model.indexes.len();
    let fields = model.fields.clone();
    let has_fields = !fields.is_empty();
    let field_api = api_base_url.clone();
    let field_application_id = application_id.clone();
    let rename_api = api_base_url.clone();
    let rename_application_id = application_id.clone();
    let model_name = model.name.clone();
    let model_title = model.title.clone();
    rsx! {
        div { class: "rounded-md border p-3",
            strong { class: "text-sm", "{model.title}" }
            form { class: "my-2 grid gap-2 sm:grid-cols-[1fr_1fr_auto]", onsubmit: move |event| {
                event.prevent_default();
                submit_patches(
                    rename_api.clone(), rename_application_id.clone(), version,
                    vec![GraphPatch::Rename {
                        target_id: model_id,
                        name: form_text(&event, "model_name"),
                        title: Some(form_text(&event, "model_title")),
                    }], generation, status,
                );
            },
                input { name: "model_name", class: "aio-input", value: model_name }
                input { name: "model_title", class: "aio-input", value: model_title }
                Button { button_type: "submit", "保存模型" }
            }
            div { class: "my-2 grid gap-1 sm:grid-cols-2 lg:grid-cols-3",
                for field in &fields {
                    div { class: "rounded border px-2 py-1 text-xs",
                        span { class: "font-medium", "{field.title}" }
                        span { class: "ml-1 text-muted-foreground", "{value_type_label(&field.value_type)}" }
                        if field.required { Badge { variant: BadgeVariant::Secondary, "必填" } }
                    }
                }
            }
            form { class: "grid gap-2 sm:grid-cols-2 lg:grid-cols-[1fr_1fr_10rem_5rem_auto]", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name");
                let title = form_text(&event, "title");
                if name.trim().is_empty() || title.trim().is_empty() { return; }
                submit_patches(
                    field_api.clone(), field_application_id.clone(), version,
                    vec![GraphPatch::Insert {
                        parent_id: model_id,
                        collection: ChildCollection::Fields,
                        index: field_count,
                        entity: GraphEntity::Field(FieldDefinition {
                            id: SymbolId::new(), name, title,
                            value_type: value_type_from_key(&form_text(&event, "value_type")),
                            state: DefinitionState::Known,
                            required: form_text(&event, "required") == "true",
                            relation_model_id: None,
                        }),
                    }], generation, status,
                );
            },
                input { name: "name", class: "aio-input", placeholder: "字段标识" }
                input { name: "title", class: "aio-input", placeholder: "字段标题" }
                {value_type_select("value_type")}
                label { class: "flex items-center gap-2 text-xs",
                    input { name: "required", r#type: "checkbox", value: "true" }
                    "必填"
                }
                Button { button_type: "submit", "添加字段" }
            }
            div { class: "mt-3 border-t pt-3",
                strong { class: "text-xs", "表达式索引" }
                for index in &model.indexes {
                    p { class: "mt-1 rounded border px-2 py-1 text-xs", "{index_purpose_label(&index.purpose)} · {index.fields.len()} 字段" }
                }
                form { class: "mt-2 grid gap-2 sm:grid-cols-[1fr_8rem_auto]", onsubmit: move |event| {
                    event.prevent_default();
                    let Ok(field_id) = SymbolId::parse(&form_text(&event, "field_id")) else { return; };
                    submit_patches(
                        api_base_url.clone(), application_id.clone(), version,
                        vec![GraphPatch::Insert {
                            parent_id: model_id,
                            collection: ChildCollection::ModelIndexes,
                            index: index_count,
                            entity: GraphEntity::ModelIndex(ModelIndexDefinition {
                                id: SymbolId::new(),
                                fields: vec![field_id],
                                purpose: index_purpose_from_key(&form_text(&event, "purpose")),
                            }),
                        }], generation, status,
                    );
                },
                    select { name: "field_id", class: "aio-input",
                        for field in fields { option { value: "{field.id}", "{field.title}" } }
                    }
                    select { name: "purpose", class: "aio-input",
                        option { value: "filter", "筛选" }
                        option { value: "sort", "排序" }
                        option { value: "relation", "关联" }
                    }
                    Button { button_type: "submit", disabled: !has_fields, "添加索引" }
                }
            }
        }
    }
}

fn index_purpose_from_key(key: &str) -> IndexPurpose {
    match key {
        "sort" => IndexPurpose::Sort,
        "relation" => IndexPurpose::Relation,
        _ => IndexPurpose::Filter,
    }
}

fn index_purpose_label(purpose: &IndexPurpose) -> &'static str {
    match purpose {
        IndexPurpose::Filter => "筛选",
        IndexPurpose::Sort => "排序",
        IndexPurpose::Relation => "关联",
    }
}

fn value_type_select(name: &'static str) -> Element {
    rsx! {
        select { name, class: "aio-input",
            option { value: "text", "文本" }
            option { value: "integer", "整数" }
            option { value: "decimal", "小数" }
            option { value: "boolean", "布尔" }
            option { value: "timestamp_ms", "时间" }
            option { value: "file", "文件" }
            option { value: "any", "任意结构" }
        }
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

fn parse_initial_value(value_type: &ValueType, raw: &str) -> serde_json::Value {
    match value_type {
        ValueType::Boolean => serde_json::Value::Bool(raw.parse().unwrap_or(false)),
        ValueType::Integer | ValueType::TimestampMs => raw
            .parse::<i64>()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
        ValueType::Decimal => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueType::Any => serde_json::from_str(raw).unwrap_or(serde_json::Value::Null),
        ValueType::Null => serde_json::Value::Null,
        _ => serde_json::Value::String(raw.to_owned()),
    }
}

fn vibe_panel(
    draft: &DraftSnapshot,
    api_base_url: String,
    mut status: Signal<Option<String>>,
) -> Element {
    let application_id = draft.application_id.clone();
    rsx! {
        Card {
            CardHeader { CardTitle { "Vibe 对话" } }
            CardContent {
                form { class: "space-y-3", onsubmit: move |event| {
                    event.prevent_default();
                    let prompt = form_text(&event, "prompt");
                    if prompt.trim().is_empty() { return; }
                    let api_base_url = api_base_url.clone();
                    let application_id = application_id.clone();
                    spawn(async move {
                        let path = format!("/api/studio/applications/{application_id}/vibe-runs");
                        match post_api::<_, VibeRunAccepted>(
                            &api_base_url,
                            &path,
                            &VibeRunRequest { prompt, model: None },
                        ).await {
                            Ok(run) => status.set(Some(format!("Vibe {}", run.status))),
                            Err(error) => status.set(Some(error)),
                        }
                    });
                },
                    textarea { name: "prompt", class: "min-h-32 w-full resize-y rounded-md border p-3 text-sm", placeholder: "描述要新建或修改的界面与交互" }
                    Button { button_type: "submit", "提交 Graph Patch" }
                }
            }
        }
    }
}

fn diagnostics_panel(draft: &DraftSnapshot) -> Element {
    let incomplete = count_incomplete(draft);
    rsx! {
        Card {
            CardHeader { CardTitle { "Draft 诊断" } }
            CardContent {
                p { class: "text-sm", "当前有 {incomplete} 个显式不完备声明。发布器还会执行符号、类型、Effect、权限、边界和 smoke test 门禁。" }
            }
        }
    }
}

fn revisions_panel(
    draft: &DraftSnapshot,
    revisions: Option<Result<Option<StudioPageData<RevisionSnapshot>>, String>>,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let application_id = draft.application_id.clone();
    match revisions {
        Some(Ok(Some(page))) => rsx! {
            div { class: "space-y-2", for revision in page.d {
                {revision_row(
                    revision,
                    api_base_url.clone(),
                    application_id.clone(),
                    generation,
                    status,
                )}
            } }
        },
        Some(Err(error)) => empty_panel(&error),
        _ => empty_panel("正在加载 revision"),
    }
}

fn revision_row(
    revision: RevisionSnapshot,
    api_base_url: String,
    application_id: String,
    mut generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let revision_id = revision.id.clone();
    rsx! {
        div { class: "flex items-center gap-3 rounded-md border p-3",
            div { class: "min-w-0 flex-1",
                strong { class: "text-sm", "Revision {revision.revision}" }
                p { class: "truncate font-mono text-xs text-muted-foreground", "{revision.content_hash}" }
            }
            Badge { variant: BadgeVariant::Outline, "{revision.origin}" }
            Button { variant: ButtonVariant::Outline, onclick: move |_| {
                let api_base_url = api_base_url.clone();
                let application_id = application_id.clone();
                let revision_id = revision_id.clone();
                spawn(async move {
                    let path = format!("/api/studio/applications/{application_id}/revisions/{revision_id}/rollback");
                    match post_api::<_, RevisionSnapshot>(&api_base_url, &path, &()).await {
                        Ok(_) => {
                            generation.with_mut(|value| *value = value.saturating_add(1));
                            status.set(Some("已回滚".to_owned()));
                        }
                        Err(error) => status.set(Some(error)),
                    }
                });
            }, "回滚" }
        }
    }
}

struct PendingStudioPatch {
    api_base_url: String,
    application_id: String,
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
    application_id: String,
    base_version: i64,
    patches: Vec<GraphPatch>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) {
    STUDIO_PATCH_QUEUE.with(|queue| {
        queue.borrow_mut().push_back(PendingStudioPatch {
            api_base_url,
            application_id,
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
            application_id,
            base_version,
            patches,
            mut generation,
            mut status,
        } = pending;
        let base_version = STUDIO_PATCH_VERSIONS.with(|versions| {
            versions
                .borrow()
                .get(&application_id)
                .copied()
                .map_or(base_version, |current| current.max(base_version))
        });
        let path = format!("/api/studio/applications/{application_id}/draft");
        let batch = GraphPatchBatch {
            base_version,
            patches,
            origin: PatchOrigin::Studio,
        };
        match patch_api::<_, DraftSnapshot>(&api_base_url, &path, &batch).await {
            Ok(draft) => {
                STUDIO_PATCH_VERSIONS.with(|versions| {
                    versions.borrow_mut().insert(application_id, draft.version);
                });
                generation.with_mut(|value| *value = value.saturating_add(1));
                status.set(Some("已保存，等待自动发布".to_owned()));
            }
            Err(error) => {
                STUDIO_PATCH_VERSIONS.with(|versions| {
                    versions.borrow_mut().remove(&application_id);
                });
                if error.starts_with("draft version conflict") {
                    STUDIO_PATCH_QUEUE.with(|queue| {
                        queue
                            .borrow_mut()
                            .retain(|item| item.application_id != application_id);
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

fn create_application(
    api_base_url: String,
    mut generation: Signal<u64>,
    mut selected_application: Signal<Option<String>>,
    mut status: Signal<Option<String>>,
) {
    let suffix = js_sys::Date::now() as u64;
    spawn(async move {
        let input = CreateApplicationInput {
            name: format!("application-{suffix}"),
            title: "新应用".to_owned(),
        };
        match post_api::<_, ApplicationSummary>(&api_base_url, "/api/studio/applications", &input)
            .await
        {
            Ok(application) => {
                selected_application.set(Some(application.id));
                generation.with_mut(|value| *value = value.saturating_add(1));
                status.set(Some("应用已创建".to_owned()));
            }
            Err(error) => status.set(Some(error)),
        }
    });
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

fn count_incomplete(draft: &DraftSnapshot) -> usize {
    let menus = draft
        .definition
        .menus
        .iter()
        .map(count_incomplete_menu)
        .sum::<usize>();
    let models = draft
        .definition
        .models
        .iter()
        .filter(|value| !value.state.is_known())
        .count();
    let pages = draft
        .definition
        .pages
        .iter()
        .filter(|value| !value.state.is_known())
        .count();
    let functions = draft
        .definition
        .functions
        .iter()
        .filter(|value| !value.state.is_known())
        .count();
    menus + models + pages + functions
}

fn count_incomplete_menu(menu: &MenuDefinition) -> usize {
    usize::from(!menu.state.is_known())
        + menu
            .children
            .iter()
            .map(count_incomplete_menu)
            .sum::<usize>()
}
