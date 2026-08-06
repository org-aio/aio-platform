use std::{convert::Infallible, fmt, str::FromStr, sync::Arc};

use crate::{
    BuiltInPageContext, BuiltInPageIndex, CompiledPageRenderer, ConventionPageContext,
    ConventionPageIndex, MenuRowActions, ProgramImage, SymbolId,
};
use crate::{PublishedProgram, WorkbenchBootstrap};
use dioxus::prelude::{
    dioxus_router::{SegmentType, SiteMapSegment},
    *,
};
use futures_util::StreamExt;
use gloo_net::eventsource::futures::EventSource;
use icons::{ListTree, PanelLeft, Plus, Settings};

use crate::{
    browser_bootstrap::{load_from_document, page_title},
    browser_http::{api_url, get_api},
    ui::StudioPage,
};

#[derive(Clone, Debug, PartialEq)]
struct AppRoute {
    path: String,
    suffix: String,
}

impl AppRoute {
    fn from_path(path: &str) -> Self {
        Self {
            path: normalize_route_path(path),
            suffix: String::new(),
        }
    }
}

impl FromStr for AppRoute {
    type Err = Infallible;

    fn from_str(route: &str) -> Result<Self, Self::Err> {
        let suffix_start = route
            .find(|character| character == '?' || character == '#')
            .unwrap_or(route.len());
        Ok(Self {
            path: normalize_route_path(&route[..suffix_start]),
            suffix: route[suffix_start..].to_owned(),
        })
    }
}

impl fmt::Display for AppRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}{}", self.path, self.suffix)
    }
}

impl Routable for AppRoute {
    const SITE_MAP: &'static [SiteMapSegment] = &[SiteMapSegment {
        segment_type: SegmentType::CatchAll("path"),
        children: &[],
    }];

    fn render(&self, _level: usize) -> Element {
        rsx! { Workbench { route: self.clone() } }
    }
}

fn normalize_route_path(path: &str) -> String {
    let normalized = if path.is_empty() || path == "/" {
        "/studio"
    } else {
        path
    };
    if normalized.starts_with('/') {
        normalized.to_owned()
    } else {
        format!("/{normalized}")
    }
}

#[allow(non_snake_case)]
pub fn App() -> Element {
    rsx! { Router::<AppRoute> {} }
}

#[component]
fn Workbench(route: AppRoute) -> Element {
    let mut bootstrap = use_signal(load_from_document);
    let route_path = route.path;
    let mut selected_scene = use_signal(|| None::<SymbolId>);
    let mut pending_scene = use_signal(|| None::<SymbolId>);
    let mut sidebar_collapsed = use_signal(|| false);
    let page_settings_open = use_signal(|| false);
    let menu_creator_open = use_signal(|| false);
    let mut scene_creator_open = use_signal(|| false);
    let mut image_generation = use_signal(|| 0_u64);
    let mut bootstrap_generation = use_signal(|| 0_u64);
    let runtime_pages = use_hook(load_runtime_page_indexes);
    let remote_bootstrap = use_resource(move || {
        let _generation = bootstrap_generation();
        async move { get_api::<WorkbenchBootstrap>("", "/api/bootstrap").await }
    });
    use_effect(move || {
        if let Some(Ok(value)) = remote_bootstrap.read().as_ref() {
            bootstrap.set(value.clone());
        }
    });
    use_effect(use_reactive(&route_path, move |route| {
        let snapshot = bootstrap();
        let selected = selected_scene();
        let pending = pending_scene();
        let next = scene_for_route(&snapshot, &route)
            .map(|(_, scene)| scene.id)
            .or_else(|| pending.filter(|id| scene_by_id(&snapshot, *id).is_some()))
            .or_else(|| selected.filter(|id| scene_by_id(&snapshot, *id).is_some()))
            .or_else(|| {
                snapshot
                    .program
                    .as_ref()
                    .and_then(|program| program.menus.first())
                    .map(|scene| scene.id)
            });
        if selected != next {
            selected_scene.set(next);
        }
        if pending.is_some() && pending == next {
            pending_scene.set(None);
        }
    }));

    let image = use_resource(use_reactive(&route_path, move |route| {
        let bootstrap = bootstrap();
        let _generation = image_generation();
        async move {
            if bootstrap.route(&route).is_none() {
                return Ok(None);
            }
            get_api::<ProgramImage>(&bootstrap.api_base_url, "/api/runtime/program/image")
                .await
                .map(Some)
        }
    }));

    let _events = use_resource(move || {
        let bootstrap = bootstrap();
        async move {
            let url = api_url(&bootstrap.api_base_url, "/api/studio/program/events");
            let Ok(mut source) = EventSource::new(&url) else {
                return;
            };
            let Ok(mut subscription) = source.subscribe("activated") else {
                return;
            };
            while subscription.next().await.is_some() {
                image_generation.with_mut(|value| *value = value.saturating_add(1));
                bootstrap_generation.with_mut(|value| *value = value.saturating_add(1));
            }
        }
    });

    let loaded_image = image.read().as_ref().cloned();
    let route = route_path;
    let snapshot = bootstrap();
    let title = page_title(&snapshot, &route);
    let editor_target = snapshot.route(&route).map(|(_, route)| route.page_id);
    let creator_target = selected_scene();
    let content = if route == "/studio" {
        rsx! {
            StudioPage {
                api_base_url: snapshot.api_base_url.clone(),
                selected_scene,
            }
        }
    } else {
        render_runtime_content(&snapshot, &route, loaded_image, runtime_pages.as_ref())
    };

    rsx! {
        document::Stylesheet { href: "/assets/dioxus-ui.css?v=91e8974" }
        document::Stylesheet { href: "/assets/app.css?v=program-runtime-13" }
        div {
            class: "aio-shell-frame bg-background text-foreground",
            "data-sidebar-collapsed": sidebar_collapsed().to_string(),
            aside { class: "aio-sidebar border-r bg-card",
                header { class: "aio-sidebar-header",
                    div { class: "aio-sidebar-header-row",
                        button {
                            class: "aio-icon-button",
                            r#type: "button",
                            title: if sidebar_collapsed() { "展开侧栏" } else { "收起侧栏" },
                            aria_label: if sidebar_collapsed() { "展开侧栏" } else { "收起侧栏" },
                            onclick: move |_| sidebar_collapsed.toggle(),
                            PanelLeft { class: "aio-sidebar-toggle-icon" }
                        }
                    }
                }
                nav { class: "aio-sidebar-scroll space-y-4", role: "menu",
                    {native_menu(
                        &snapshot,
                        &route,
                        page_settings_open,
                        menu_creator_open,
                        creator_target.is_some(),
                    )}
                    if let Some((program, scene)) = selected_scene()
                        .and_then(|scene_id| scene_by_id(&snapshot, scene_id))
                    {
                        {scene_menu(program, scene, &route)}
                    }
                }
            }
            main { class: "aio-main min-w-0",
                header { class: "aio-topbar border-b bg-background/95 backdrop-blur",
                    div { class: "aio-topbar-title min-w-0",
                        h1 { class: "truncate text-sm font-semibold", "{title}" }
                    }
                    nav { class: "aio-root-menu", aria_label: "场景",
                        if let Some(program) = &snapshot.program {
                            for scene in &program.menus {
                                {scene_link(
                                    scene,
                                    program,
                                    selected_scene() == Some(scene.id),
                                    selected_scene,
                                )}
                            }
                        }
                        if snapshot.admin.as_ref().is_some_and(|state| state.can_add_scene) {
                            button {
                                class: "aio-root-menu-add",
                                r#type: "button",
                                title: "添加场景",
                                aria_label: "添加场景",
                                onclick: move |_| scene_creator_open.set(true),
                                Plus { class: "size-4" }
                            }
                        }
                    }
                }
                section { class: "aio-main-scroll bg-muted/30",
                    {content}
                }
            }
            if page_settings_open() {
                if let Some(page_id) = editor_target {
                    crate::ui::AdminPageEditor {
                        api_base_url: snapshot.api_base_url.clone(),
                        page_id,
                        settings_open: page_settings_open,
                    }
                }
            }
            if menu_creator_open() {
                if let Some(scene_id) = creator_target {
                    crate::ui::AdminMenuCreator {
                        api_base_url: snapshot.api_base_url.clone(),
                        scene_id,
                        creator_open: menu_creator_open,
                    }
                }
            }
            if scene_creator_open() {
                crate::ui::AdminSceneCreator {
                    api_base_url: snapshot.api_base_url.clone(),
                    pending_scene,
                    creator_open: scene_creator_open,
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct RuntimePageIndexes {
    convention: ConventionPageIndex,
    built_in: BuiltInPageIndex,
}

fn load_runtime_page_indexes() -> Result<Arc<RuntimePageIndexes>, String> {
    let mut context = rudi::Context::auto_register();
    let convention =
        ConventionPageIndex::from_context(&mut context).map_err(|error| error.to_string())?;
    let built_in =
        BuiltInPageIndex::from_context(&mut context).map_err(|error| error.to_string())?;
    Ok(Arc::new(RuntimePageIndexes {
        convention,
        built_in,
    }))
}

fn render_runtime_content(
    bootstrap: &WorkbenchBootstrap,
    route: &str,
    image: Option<Result<Option<ProgramImage>, String>>,
    runtime_pages: Result<&Arc<RuntimePageIndexes>, &String>,
) -> Element {
    let Some((program, compiled_route)) = bootstrap.route(route) else {
        return error_state("路由不存在", route);
    };
    let api_base_url = bootstrap.api_base_url.clone();
    let image = match image {
        Some(Ok(Some(image))) => image,
        Some(Ok(None)) => return error_state("活动版本不存在", "Program 尚未发布"),
        Some(Err(error)) => return error_state("加载 ProgramImage 失败", &error),
        None => return loading_state(),
    };
    let Some(page) = image.pages.get(&compiled_route.page_id).cloned() else {
        return error_state("页面编译产物不存在", &compiled_route.page_id.to_string());
    };
    let renderer = page.renderer.clone();
    match renderer {
        CompiledPageRenderer::ConventionFile {
            module_name,
            expected_path,
        } => {
            let indexes = match runtime_pages {
                Ok(indexes) => indexes,
                Err(error) => return error_state("约定页面 Provider 初始化失败", error),
            };
            indexes
                .convention
                .render(
                    &module_name,
                    ConventionPageContext {
                        route: route.to_owned(),
                        page,
                    },
                )
                .unwrap_or_else(|| {
                    error_state(
                        "约定页面文件尚未进入构建",
                        &format!("期望文件: app/{expected_path}"),
                    )
                })
        }
        CompiledPageRenderer::TreeTable { provider_key, .. }
        | CompiledPageRenderer::CrudTable { provider_key, .. } => {
            let indexes = match runtime_pages {
                Ok(indexes) => indexes,
                Err(error) => return error_state("内置页面 Provider 初始化失败", error),
            };
            let row_actions =
                row_actions_for_page(&program.menus, compiled_route.page_id).unwrap_or_default();
            indexes
                .built_in
                .render(
                    &provider_key,
                    BuiltInPageContext {
                        api_base_url,
                        image,
                        page,
                        row_actions,
                    },
                )
                .unwrap_or_else(|| {
                    error_state(
                        "内置页面 Provider 未注册",
                        &format!("Rudi Provider: {provider_key}"),
                    )
                })
        }
    }
}

fn row_actions_for_page(
    menus: &[crate::MenuDefinition],
    page_id: SymbolId,
) -> Option<MenuRowActions> {
    menus.iter().find_map(|menu| {
        if menu.page_id == Some(page_id) {
            return Some(menu.row_actions.clone());
        }
        row_actions_for_page(&menu.children, page_id)
    })
}

fn native_menu(
    bootstrap: &WorkbenchBootstrap,
    active_route: &str,
    mut page_settings_open: Signal<bool>,
    mut menu_creator_open: Signal<bool>,
    has_selected_scene: bool,
) -> Element {
    let admin = bootstrap.admin.clone();
    let has_current_page = bootstrap.route(active_route).is_some();
    let can_edit_current =
        has_current_page && admin.as_ref().is_some_and(|state| state.can_edit_page);
    let can_add_menu = admin.as_ref().is_some_and(|state| state.can_add_menu);
    let can_manage_menus = admin.is_some();
    rsx! {
        section { class: "space-y-1",
            div { class: "aio-sidebar-section-heading",
                p { class: "aio-sidebar-section-title text-xs font-semibold uppercase text-muted-foreground", "管理工具" }
                div { class: "aio-sidebar-section-actions",
                    if can_manage_menus {
                        Link {
                            class: if active_route == "/studio" {
                                "aio-sidebar-admin-action aio-sidebar-admin-action--primary"
                            } else {
                                "aio-sidebar-admin-action"
                            },
                            to: AppRoute::from_path("/studio"),
                            title: "管理场景与菜单",
                            aria_label: "管理场景与菜单",
                            ListTree { class: "size-4" }
                        }
                    }
                    if can_add_menu {
                        button {
                            class: "aio-sidebar-admin-action aio-sidebar-admin-action--primary",
                            r#type: "button",
                            disabled: !has_selected_scene,
                            title: if has_selected_scene { "添加菜单" } else { "请先添加场景" },
                            aria_label: if has_selected_scene { "添加菜单" } else { "请先添加场景" },
                            onclick: move |_| menu_creator_open.set(true),
                            Plus { class: "size-4" }
                        }
                    }
                    if can_edit_current {
                        button {
                            class: "aio-sidebar-admin-action",
                            r#type: "button",
                            title: "页面设置",
                            aria_label: "页面设置",
                            onclick: move |_| page_settings_open.set(true),
                            Settings { class: "size-4" }
                        }
                    }
                }
            }
            for entry in &bootstrap.native_entries {
                {menu_link(&entry.route, &entry.title, "◇", active_route)}
            }
        }
    }
}

fn scene_menu(
    program: &PublishedProgram,
    scene: &crate::MenuDefinition,
    active_route: &str,
) -> Element {
    rsx! {
        section { class: "space-y-1",
            for menu in &scene.children {
                {program_menu(menu, program, active_route)}
            }
        }
    }
}

fn scene_link(
    scene: &crate::MenuDefinition,
    program: &PublishedProgram,
    active: bool,
    mut selected_scene: Signal<Option<SymbolId>>,
) -> Element {
    let scene_id = scene.id;
    let title = scene.title.clone();
    let route = first_menu_route(scene, program);
    let class = if active {
        "aio-root-menu-item aio-root-menu-item--active"
    } else {
        "aio-root-menu-item"
    };
    match route {
        Some(route) => rsx! {
            Link {
                class,
                to: AppRoute::from_path(&route),
                onclick: move |_| selected_scene.set(Some(scene_id)),
                "{title}"
            }
        },
        None => rsx! {
            button {
                class,
                r#type: "button",
                onclick: move |_| selected_scene.set(Some(scene_id)),
                "{title}"
            }
        },
    }
}

fn scene_by_id<'a>(
    bootstrap: &'a WorkbenchBootstrap,
    scene_id: SymbolId,
) -> Option<(&'a PublishedProgram, &'a crate::MenuDefinition)> {
    let program = bootstrap.program.as_ref()?;
    program
        .menus
        .iter()
        .find(|scene| scene.id == scene_id)
        .map(|scene| (program, scene))
}

fn scene_for_route<'a>(
    bootstrap: &'a WorkbenchBootstrap,
    path: &str,
) -> Option<(&'a PublishedProgram, &'a crate::MenuDefinition)> {
    let (program, route) = bootstrap.route(path)?;
    program
        .menus
        .iter()
        .find(|scene| menu_contains_page(scene, route.page_id))
        .map(|scene| (program, scene))
}

fn menu_contains_page(menu: &crate::MenuDefinition, page_id: SymbolId) -> bool {
    menu.page_id == Some(page_id)
        || menu
            .children
            .iter()
            .any(|child| menu_contains_page(child, page_id))
}

fn first_menu_route(menu: &crate::MenuDefinition, program: &PublishedProgram) -> Option<String> {
    menu.page_id
        .and_then(|page_id| {
            program
                .routes
                .iter()
                .find(|route| route.page_id == page_id)
                .map(|route| route.path.clone())
        })
        .or_else(|| {
            menu.children
                .iter()
                .find_map(|child| first_menu_route(child, program))
        })
}

fn program_menu(
    menu: &crate::MenuDefinition,
    program: &PublishedProgram,
    active_route: &str,
) -> Element {
    let route = menu.page_id.and_then(|page_id| {
        program
            .routes
            .iter()
            .find(|route| route.page_id == page_id)
            .map(|route| route.path.clone())
    });
    let icon = menu.icon.as_deref().unwrap_or("□");
    rsx! {
        div { class: "space-y-1",
            if let Some(route) = route {
                {menu_link(&route, &menu.title, icon, active_route)}
            } else {
                div { class: "px-3 py-2 text-sm font-medium", "{icon}  {menu.title}" }
            }
            if !menu.children.is_empty() {
                div { class: "ml-3 space-y-1 border-l pl-2",
                    for child in &menu.children {
                        {program_menu(child, program, active_route)}
                    }
                }
            }
        }
    }
}

fn menu_link(route: &str, label: &str, icon: &str, active_route: &str) -> Element {
    let class = if route == active_route {
        "aio-sidebar-menu-link flex items-center gap-2 rounded-md bg-muted px-3 py-2 text-sm text-foreground"
    } else {
        "aio-sidebar-menu-link flex items-center gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
    };
    rsx! {
        Link {
            class,
            to: AppRoute::from_path(route),
            span { class: "aio-sidebar-menu-icon w-4 shrink-0 text-center", "{icon}" }
            span { class: "aio-sidebar-menu-label min-w-0 truncate", "{label}" }
        }
    }
}

fn loading_state() -> Element {
    rsx! { div { class: "flex items-center gap-2 p-6 text-sm text-muted-foreground", "正在加载活动版本…" } }
}

fn error_state(title: &str, message: &str) -> Element {
    rsx! {
        div { class: "rounded-md border border-destructive bg-destructive/10 p-4 text-destructive",
            strong { class: "text-sm", "{title}" }
            p { class: "mt-1 text-sm break-words", "{message}" }
        }
    }
}
