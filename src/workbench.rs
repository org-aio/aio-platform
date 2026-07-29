use std::{collections::BTreeMap, sync::Arc};

use crate::{
    ApplicationImage, GraphVm, GraphVmHost, SegmentInvocationRequest, SegmentInvocationResult,
    SymbolId, VmEffect,
};
use crate::{ComponentIndex, DynamicComponentEvent, DynamicRenderData, DynamicRenderer};
use crate::{PublishedApplication, WorkbenchBootstrap};
use dioxus::prelude::*;
use futures_util::StreamExt;
use gloo_net::eventsource::futures::EventSource;
use serde_json::Value;

use crate::design_system::{Badge, BadgeVariant};
use crate::{
    browser_bootstrap::{initial_route, load_from_document, page_title, push_route},
    browser_http::{api_url, get_api, post_api},
    ui::StudioPage,
};

#[allow(non_snake_case)]
pub fn App() -> Element {
    let mut bootstrap = use_signal(load_from_document);
    let initial_route = initial_route(&bootstrap.read());
    let active_route = use_signal(move || initial_route);
    let mut selected_application = use_signal(|| None::<String>);
    let mut selected_scene = use_signal(|| None::<SymbolId>);
    let mut image_generation = use_signal(|| 0_u64);
    let mut bootstrap_generation = use_signal(|| 0_u64);
    let mut page_state = use_signal(BTreeMap::<SymbolId, Value>::new);
    let mut notification = use_signal(|| None::<String>);
    let component_index = use_hook(load_component_index);
    let remote_bootstrap = use_resource(move || {
        let _generation = bootstrap_generation();
        async move { get_api::<WorkbenchBootstrap>("", "/api/bootstrap").await }
    });
    use_effect(move || {
        if let Some(Ok(value)) = remote_bootstrap.read().as_ref() {
            bootstrap.set(value.clone());
        }
    });
    use_effect(move || {
        let snapshot = bootstrap();
        let route = active_route();
        let selected_application_id = selected_application();
        let selected = selected_scene();
        let route_application = snapshot
            .route(&route)
            .map(|(application, _)| application.application_id.clone());
        let next_application = route_application
            .or_else(|| {
                selected_application_id.filter(|id| application_by_id(&snapshot, id).is_some())
            })
            .or_else(|| {
                snapshot
                    .applications
                    .first()
                    .map(|application| application.application_id.clone())
            });
        if selected_application() != next_application {
            selected_application.set(next_application.clone());
        }
        let next = scene_for_route(&snapshot, &route)
            .filter(|(application, _)| {
                Some(&application.application_id) == next_application.as_ref()
            })
            .map(|(_, scene)| scene.id)
            .or_else(|| {
                selected.filter(|id| {
                    next_application.as_deref().is_some_and(|application_id| {
                        scene_by_id(&snapshot, application_id, *id).is_some()
                    })
                })
            })
            .or_else(|| {
                next_application.as_deref().and_then(|application_id| {
                    application_by_id(&snapshot, application_id)
                        .and_then(|application| application.menus.first())
                        .map(|scene| scene.id)
                })
            });
        if selected != next {
            selected_scene.set(next);
        }
    });

    let image = use_resource(move || {
        let route = active_route();
        let bootstrap = bootstrap();
        let _generation = image_generation();
        async move {
            let Some((application, _)) = bootstrap.route(&route) else {
                return Ok(None);
            };
            let path = format!(
                "/api/runtime/applications/{}/image",
                application.application_id
            );
            get_api::<ApplicationImage>(&bootstrap.api_base_url, &path)
                .await
                .map(Some)
        }
    });

    let _events = use_resource(move || {
        let route = active_route();
        let bootstrap = bootstrap();
        async move {
            let Some((application, _)) = bootstrap.route(&route) else {
                return;
            };
            let url = api_url(
                &bootstrap.api_base_url,
                &format!(
                    "/api/studio/applications/{}/events",
                    application.application_id
                ),
            );
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
    use_effect(move || {
        if let Some(Ok(Some(image))) = image.read().as_ref() {
            let route = active_route();
            if let Some(page) = page_for_route(image, &route) {
                page_state.set(page.page_state.clone());
            }
        }
    });

    let route = active_route();
    use_effect(move || {
        let _route = active_route();
        notification.set(None);
    });
    let snapshot = bootstrap();
    let title = page_title(&snapshot, &route);
    let content = if route == "/studio" {
        rsx! {
            StudioPage {
                api_base_url: snapshot.api_base_url.clone(),
                selected_application,
            }
        }
    } else {
        render_runtime_content(
            &snapshot,
            &route,
            loaded_image,
            component_index.as_ref(),
            active_route,
            page_state,
            notification,
        )
    };

    rsx! {
        document::Stylesheet { href: "/assets/dioxus-ui.css?v=91e8974" }
        document::Stylesheet { href: "/assets/app.css?v=program-runtime-6" }
        div { class: "aio-shell-frame bg-background text-foreground",
            aside { class: "aio-sidebar border-r bg-card",
                header { class: "aio-sidebar-header",
                    div { class: "aio-sidebar-brand px-3 py-2",
                        p { class: "aio-sidebar-brand-title", "AIO" }
                        p { class: "aio-sidebar-brand-subtitle", "ProgramGraph Workbench" }
                    }
                }
                nav { class: "aio-sidebar-scroll space-y-4", role: "menu",
                    {native_menu(&snapshot, &route, active_route)}
                    if let Some((application, scene)) = selected_application()
                        .as_deref()
                        .and_then(|application_id| selected_scene().and_then(|scene_id| {
                            scene_by_id(&snapshot, application_id, scene_id)
                        }))
                    {
                        {scene_menu(application, scene, &route, active_route)}
                    }
                }
            }
            main { class: "aio-main min-w-0",
                header { class: "aio-topbar border-b bg-background/95 backdrop-blur",
                    div { class: "aio-topbar-title min-w-0",
                        h1 { class: "truncate text-sm font-semibold", "{title}" }
                    }
                    nav { class: "aio-root-menu", aria_label: "场景",
                        if let Some(application) = selected_application()
                            .as_deref()
                            .and_then(|application_id| application_by_id(&snapshot, application_id))
                        {
                            for scene in &application.menus {
                                {scene_link(
                                    scene,
                                    application,
                                    selected_scene() == Some(scene.id),
                                    active_route,
                                    selected_scene,
                                )}
                            }
                        }
                    }
                    div { class: "aio-toolbar-actions",
                        Badge { variant: BadgeVariant::Outline, "{route}" }
                    }
                }
                section { class: "aio-main-scroll bg-muted/30",
                    if let Some(message) = notification() {
                        div { class: "mb-3 rounded-md border bg-background p-3 text-sm break-words", "{message}" }
                    }
                    {content}
                }
            }
        }
    }
}

fn load_component_index() -> Result<Arc<ComponentIndex>, String> {
    let mut context = rudi::Context::auto_register();
    ComponentIndex::from_context(&mut context)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

fn render_runtime_content(
    bootstrap: &WorkbenchBootstrap,
    route: &str,
    image: Option<Result<Option<ApplicationImage>, String>>,
    component_index: Result<&Arc<ComponentIndex>, &String>,
    active_route: Signal<String>,
    page_state: Signal<BTreeMap<SymbolId, Value>>,
    mut notification: Signal<Option<String>>,
) -> Element {
    let Some((application, compiled_route)) = bootstrap.route(route) else {
        return error_state("路由不存在", route);
    };
    let application_id = application.application_id.clone();
    let api_base_url = bootstrap.api_base_url.clone();
    let routes = application
        .routes
        .iter()
        .map(|route| (route.id, route.path.clone()))
        .collect::<BTreeMap<_, _>>();
    let image = match image {
        Some(Ok(Some(image))) => image,
        Some(Ok(None)) => return error_state("活动版本不存在", &application_id),
        Some(Err(error)) => return error_state("加载 ApplicationImage 失败", &error),
        None => return loading_state(),
    };
    let Some(plan) = image.pages.get(&compiled_route.page_id) else {
        return error_state("RenderPlan 不存在", &compiled_route.page_id.to_string());
    };
    let components = match component_index {
        Ok(value) => Arc::clone(value),
        Err(error) => return error_state("组件 Provider 初始化失败", error),
    };
    let plan = plan.clone();
    let event_plan = plan.clone();
    let image_for_event = image.clone();
    let dispatch = Callback::new(move |event: DynamicComponentEvent| {
        let Some(function_id) = find_event_function(&event_plan.root, &event) else {
            notification.set(Some(format!(
                "组件事件未绑定函数: {}.{}",
                event.component_id, event.event
            )));
            return;
        };
        let Some(segment) = image_for_event
            .client_functions
            .get(&function_id)
            .or_else(|| image_for_event.server_functions.get(&function_id))
        else {
            notification.set(Some(format!("发布函数不存在: {function_id}")));
            return;
        };
        let inputs = segment
            .input_ports
            .iter()
            .map(|(id, name)| (*id, event.payload.get(name).cloned().unwrap_or(Value::Null)))
            .collect::<BTreeMap<_, _>>();
        let image = image_for_event.clone();
        let application_id = application_id.clone();
        let api_base_url = api_base_url.clone();
        let routes = routes.clone();
        spawn(async move {
            let result = if image.client_functions.contains_key(&function_id) {
                let mut host = ClientVmHost {
                    active_route,
                    page_state,
                    notification,
                    routes,
                    api_base_url,
                    application_id,
                };
                GraphVm::new(&image.client_functions)
                    .execute(function_id, &inputs, &mut host)
                    .await
            } else {
                let path =
                    format!("/api/runtime/applications/{application_id}/segments/{function_id}");
                post_api::<_, SegmentInvocationResult>(
                    &api_base_url,
                    &path,
                    &SegmentInvocationRequest { inputs },
                )
                .await
                .map(|result| result.value)
                .map_err(anyhow::Error::msg)
            };
            if let Err(error) = result {
                notification.set(Some(format!("交互执行失败: {error:#}")));
            }
        });
    });
    let data = DynamicRenderData {
        page_state: page_state(),
        ..DynamicRenderData::default()
    };
    DynamicRenderer::new(components).render(&plan, &data, dispatch)
}

fn page_for_route<'a>(image: &'a ApplicationImage, route: &str) -> Option<&'a crate::RenderPlan> {
    let page_id = image
        .routes
        .iter()
        .find(|compiled| compiled.path == route)
        .map(|compiled| compiled.page_id)?;
    image.pages.get(&page_id)
}

fn find_event_function(
    node: &crate::RenderNode,
    event: &DynamicComponentEvent,
) -> Option<SymbolId> {
    if node.id == event.component_id {
        return node.events.get(&event.event).copied();
    }
    node.children
        .iter()
        .find_map(|child| find_event_function(child, event))
}

fn native_menu(
    bootstrap: &WorkbenchBootstrap,
    active_route: &str,
    active_route_signal: Signal<String>,
) -> Element {
    rsx! {
        section { class: "space-y-1",
            p { class: "aio-sidebar-section-title px-3 text-xs font-semibold uppercase text-muted-foreground", "母机" }
            for entry in &bootstrap.native_entries {
                {menu_link(&entry.route, &entry.title, "◇", active_route, active_route_signal)}
            }
        }
    }
}

fn scene_menu(
    application: &PublishedApplication,
    scene: &crate::MenuDefinition,
    active_route: &str,
    active_route_signal: Signal<String>,
) -> Element {
    rsx! {
        section { class: "space-y-1",
            for menu in &scene.children {
                {program_menu(menu, application, active_route, active_route_signal)}
            }
        }
    }
}

fn scene_link(
    scene: &crate::MenuDefinition,
    application: &PublishedApplication,
    active: bool,
    mut active_route: Signal<String>,
    mut selected_scene: Signal<Option<SymbolId>>,
) -> Element {
    let scene_id = scene.id;
    let title = scene.title.clone();
    let route = first_menu_route(scene, application);
    let class = if active {
        "aio-root-menu-item aio-root-menu-item--active"
    } else {
        "aio-root-menu-item"
    };
    rsx! {
        button {
            class,
            r#type: "button",
            onclick: move |_| {
                selected_scene.set(Some(scene_id));
                if let Some(route) = &route {
                    active_route.set(route.clone());
                    push_route(route);
                }
            },
            "{title}"
        }
    }
}

fn application_by_id<'a>(
    bootstrap: &'a WorkbenchBootstrap,
    application_id: &str,
) -> Option<&'a PublishedApplication> {
    bootstrap
        .applications
        .iter()
        .find(|application| application.application_id == application_id)
}

fn scene_by_id<'a>(
    bootstrap: &'a WorkbenchBootstrap,
    application_id: &str,
    scene_id: SymbolId,
) -> Option<(&'a PublishedApplication, &'a crate::MenuDefinition)> {
    let application = application_by_id(bootstrap, application_id)?;
    application
        .menus
        .iter()
        .find(|scene| scene.id == scene_id)
        .map(|scene| (application, scene))
}

fn scene_for_route<'a>(
    bootstrap: &'a WorkbenchBootstrap,
    path: &str,
) -> Option<(&'a PublishedApplication, &'a crate::MenuDefinition)> {
    let (application, route) = bootstrap.route(path)?;
    application
        .menus
        .iter()
        .find(|scene| menu_contains_page(scene, route.page_id))
        .map(|scene| (application, scene))
}

fn menu_contains_page(menu: &crate::MenuDefinition, page_id: SymbolId) -> bool {
    menu.page_id == Some(page_id)
        || menu
            .children
            .iter()
            .any(|child| menu_contains_page(child, page_id))
}

fn first_menu_route(
    menu: &crate::MenuDefinition,
    application: &PublishedApplication,
) -> Option<String> {
    menu.page_id
        .and_then(|page_id| {
            application
                .routes
                .iter()
                .find(|route| route.page_id == page_id)
                .map(|route| route.path.clone())
        })
        .or_else(|| {
            menu.children
                .iter()
                .find_map(|child| first_menu_route(child, application))
        })
}

fn program_menu(
    menu: &crate::MenuDefinition,
    application: &PublishedApplication,
    active_route: &str,
    active_route_signal: Signal<String>,
) -> Element {
    let route = menu.page_id.and_then(|page_id| {
        application
            .routes
            .iter()
            .find(|route| route.page_id == page_id)
            .map(|route| route.path.clone())
    });
    let icon = menu.icon.as_deref().unwrap_or("□");
    rsx! {
        div { class: "space-y-1",
            if let Some(route) = route {
                {menu_link(&route, &menu.title, icon, active_route, active_route_signal)}
            } else {
                div { class: "px-3 py-2 text-sm font-medium", "{icon}  {menu.title}" }
            }
            if !menu.children.is_empty() {
                div { class: "ml-3 space-y-1 border-l pl-2",
                    for child in &menu.children {
                        {program_menu(child, application, active_route, active_route_signal)}
                    }
                }
            }
        }
    }
}

fn menu_link(
    route: &str,
    label: &str,
    icon: &str,
    active_route: &str,
    mut active_route_signal: Signal<String>,
) -> Element {
    let class = if route == active_route {
        "aio-sidebar-menu-link flex items-center gap-2 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground"
    } else {
        "aio-sidebar-menu-link flex items-center gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground hover:bg-accent"
    };
    let route = route.to_owned();
    rsx! {
        a {
            class,
            href: format!("?route={route}"),
            onclick: move |event| {
                event.prevent_default();
                active_route_signal.set(route.clone());
                push_route(&route);
            },
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

struct ClientVmHost {
    active_route: Signal<String>,
    page_state: Signal<BTreeMap<SymbolId, Value>>,
    notification: Signal<Option<String>>,
    routes: BTreeMap<SymbolId, String>,
    api_base_url: String,
    application_id: String,
}

impl GraphVmHost for ClientVmHost {
    async fn apply(&mut self, effect: VmEffect) -> anyhow::Result<Value> {
        match effect {
            VmEffect::SetState { state_id, value } => {
                self.page_state.write().insert(state_id, value.clone());
                Ok(value)
            }
            VmEffect::Navigate { route_id } => {
                let route = self
                    .routes
                    .get(&route_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("路由不存在: {route_id}"))?;
                self.active_route.set(route.clone());
                push_route(&route);
                Ok(Value::String(route))
            }
            VmEffect::Confirm { value } => {
                let message = value.as_str().unwrap_or("确认执行此操作？");
                let confirmed = web_sys::window()
                    .and_then(|window| window.confirm_with_message(message).ok())
                    .unwrap_or(false);
                Ok(Value::Bool(confirmed))
            }
            VmEffect::OpenDialog { component_id } => {
                self.page_state
                    .write()
                    .insert(component_id, Value::Bool(true));
                Ok(Value::Bool(true))
            }
            VmEffect::CloseDialog { component_id } => {
                self.page_state
                    .write()
                    .insert(component_id, Value::Bool(false));
                Ok(Value::Bool(false))
            }
            VmEffect::Notify { level, value } => {
                self.notification
                    .set(Some(format!("{level}: {}", value_text(&value))));
                Ok(value)
            }
            VmEffect::Refresh { .. } | VmEffect::ValidateForm { .. } => Ok(Value::Null),
            VmEffect::InvokeServerSegment { segment_id, inputs } => {
                let path = format!(
                    "/api/runtime/applications/{}/segments/{segment_id}",
                    self.application_id
                );
                post_api::<_, SegmentInvocationResult>(
                    &self.api_base_url,
                    &path,
                    &SegmentInvocationRequest { inputs },
                )
                .await
                .map(|result| result.value)
                .map_err(anyhow::Error::msg)
            }
            VmEffect::CreateRecord { .. }
            | VmEffect::ReadRecord { .. }
            | VmEffect::UpdateRecord { .. }
            | VmEffect::DeleteRecord { .. }
            | VmEffect::QueryRecords { .. }
            | VmEffect::Capability { .. } => {
                anyhow::bail!("服务端 Effect 不能在客户端 VM 执行")
            }
        }
    }
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}
