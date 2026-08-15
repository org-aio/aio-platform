use dioxus::prelude::*;
#[cfg(target_arch = "wasm32")]
use futures_util::StreamExt;
#[cfg(target_arch = "wasm32")]
use gloo_net::eventsource::futures::EventSource;

use crate::{
    BuiltInPage, CompiledPage, CompiledPageRenderer, ConventionPageContext, EndpointWorkbench,
    MenuDefinition, MenuRowActions, ProgramImage, SymbolId, browser_http::get_api,
};
pub use az_dioxus_admin_shell::ApplicationUser;
use az_dioxus_admin_shell::{
    ApplicationAccountAction, ApplicationMenuItem, ApplicationSceneItem, ApplicationShell,
};

type ConventionPageRenderFn = fn(SymbolId) -> Element;

#[derive(Clone, Copy)]
pub struct ConventionPageRenderer(ConventionPageRenderFn);

impl ConventionPageRenderer {
    #[must_use]
    pub const fn new(render: ConventionPageRenderFn) -> Self {
        Self(render)
    }

    #[must_use]
    pub const fn endpoint_page() -> Self {
        Self(render_endpoint_convention_page)
    }

    fn render(self, page_id: SymbolId) -> Element {
        (self.0)(page_id)
    }
}

fn render_endpoint_convention_page(page_id: SymbolId) -> Element {
    rsx! {
        crate::EndpointPage { page_id: page_id.to_string() }
    }
}

impl PartialEq for ConventionPageRenderer {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

/// Web 与 Desktop 共用的发布应用壳，页面内容完全由 ProgramImage 决定。
#[component]
pub fn PublishedApplication(
    render_convention: ConventionPageRenderer,
    admin_enabled: bool,
    user: ApplicationUser,
) -> Element {
    let image_generation = use_signal(|| 0_u64);
    let mut status = use_signal(|| None::<String>);
    use_program_activation_refresh(image_generation, status);
    let image = use_resource(move || async move {
        let _generation = image_generation();
        get_api::<ProgramImage>("", "/api/runtime/program/image").await
    });
    let mut selected_page = use_signal(|| None::<SymbolId>);
    let mut selected_scene = use_signal(|| None::<SymbolId>);
    let mut settings_open = use_signal(|| false);
    let mut application_editor_open = use_signal(|| false);
    let mut scene_creator_open = use_signal(|| false);
    let mut deleting_scene = use_signal(|| None::<SymbolId>);
    let mut menu_creator_open = use_signal(|| false);
    let Some(result) = image.read().as_ref().cloned() else {
        return state("正在加载应用", false);
    };
    let image = match result {
        Ok(image) => image,
        Err(error) => return state(&error, true),
    };
    let selected_page_value = selected_page().filter(|page_id| image.pages.contains_key(page_id));
    let active_scene = selected_scene()
        .filter(|scene_id| enabled_scene(&image.menus, *scene_id).is_some())
        .or_else(|| {
            selected_page_value.and_then(|page_id| scene_id_for_page(&image.menus, page_id))
        })
        .or_else(|| first_enabled_scene(&image.menus));
    let active_scene_definition =
        active_scene.and_then(|scene_id| enabled_scene(&image.menus, scene_id));
    let active_page = selected_page_value
        .filter(|page_id| {
            active_scene_definition.is_some_and(|scene| menu_contains_page(scene, *page_id))
        })
        .or_else(|| active_scene_definition.and_then(first_visible_page_in_scene));
    let page = active_page.and_then(|page_id| image.pages.get(&page_id).cloned());
    let active_title = page
        .as_ref()
        .map(|page| page.title.clone())
        .unwrap_or_else(|| "请选择页面".to_owned());
    let active_menu = active_page.and_then(|page_id| {
        active_scene_definition.and_then(|scene| menu_id_for_page(scene, page_id))
    });
    let content = if let Some(page) = page {
        render_compiled_page(&image, page, render_convention)
    } else {
        state("请选择页面", false)
    };
    let application_scenes = application_scenes(&image.menus);
    let application_menus = active_scene_definition
        .map(application_menus)
        .unwrap_or_default();
    let select_scene = Callback::new(move |scene_id: String| match SymbolId::parse(&scene_id) {
        Ok(scene_id) => {
            selected_scene.set(Some(scene_id));
            selected_page.set(None);
            settings_open.set(false);
            menu_creator_open.set(false);
            status.set(None);
        }
        Err(error) => status.set(Some(error.to_string())),
    });
    let select_page = Callback::new(move |page_id: String| match SymbolId::parse(&page_id) {
        Ok(page_id) => {
            selected_page.set(Some(page_id));
            settings_open.set(false);
            status.set(None);
        }
        Err(error) => status.set(Some(error.to_string())),
    });
    let create_menu =
        (admin_enabled && active_scene.is_some()).then_some(Callback::new(move |()| {
            status.set(None);
            menu_creator_open.set(true);
        }));
    let create_scene = admin_enabled.then_some(Callback::new(move |()| {
        status.set(None);
        scene_creator_open.set(true);
    }));
    let edit_application = admin_enabled.then_some(Callback::new(move |()| {
        status.set(None);
        application_editor_open.set(true);
    }));
    let delete_scene =
        admin_enabled.then_some(Callback::new(
            move |scene_id: String| match SymbolId::parse(&scene_id) {
                Ok(scene_id) => {
                    status.set(None);
                    deleting_scene.set(Some(scene_id));
                }
                Err(error) => status.set(Some(error.to_string())),
            },
        ));
    let configure_page =
        (admin_enabled && active_page.is_some()).then_some(Callback::new(move |()| {
            status.set(None);
            settings_open.set(true);
        }));
    let account_action = Callback::new(move |action| {
        let message = match action {
            ApplicationAccountAction::AgentSettings => "Agent 设置尚未接入",
            ApplicationAccountAction::Profile => "个人资料尚未接入",
            ApplicationAccountAction::ChangePassword => "修改密码尚未接入",
            ApplicationAccountAction::SignOut => "退出系统尚未接入",
        };
        status.set(Some(message.to_owned()));
    });

    rsx! {
        ApplicationShell {
            application_label: image.title.clone(),
            page_label: active_title,
            scenes: application_scenes,
            active_scene_id: active_scene.map(|scene_id| scene_id.to_string()),
            menus: application_menus,
            active_page_id: active_page.map(|page_id| page_id.to_string()),
            user,
            status: status(),
            on_select_scene: select_scene,
            on_select_page: select_page,
            on_edit_application: edit_application,
            on_create_scene: create_scene,
            on_delete_scene: delete_scene,
            on_create_menu: create_menu,
            on_configure_page: configure_page,
            on_account_action: account_action,
            {content}
        }
        if admin_enabled && application_editor_open() {
            crate::ui::AdminApplicationTitleEditor {
                api_base_url: String::new(),
                current_title: image.title.clone(),
                generation: image_generation,
                status,
                editor_open: application_editor_open,
            }
        }
        if admin_enabled
            && settings_open()
            && let Some(page_id) = active_page
        {
            crate::ui::AdminPageEditor {
                api_base_url: String::new(),
                page_id,
                menu_id: active_menu,
                generation: image_generation,
                status,
                settings_open,
            }
        }
        if admin_enabled && scene_creator_open() {
            crate::ui::AdminSceneCreator {
                api_base_url: String::new(),
                pending_scene: selected_scene,
                creator_open: scene_creator_open,
                shared_generation: Some(image_generation),
                shared_status: Some(status),
            }
        }
        if admin_enabled
            && let Some(scene_id) = deleting_scene()
        {
            crate::ui::AdminSceneDeleteDialog {
                api_base_url: String::new(),
                scene_id,
                generation: image_generation,
                status,
                deleting_scene,
                on_deleted: move |_| {
                    selected_scene.set(None);
                    selected_page.set(None);
                },
            }
        }
        if admin_enabled
            && menu_creator_open()
            && let Some(scene_id) = active_scene
        {
            crate::ui::AdminMenuCreator {
                api_base_url: String::new(),
                scene_id,
                generation: image_generation,
                status,
                creator_open: menu_creator_open,
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn use_program_activation_refresh(mut generation: Signal<u64>, mut status: Signal<Option<String>>) {
    use_effect(move || {
        spawn(async move {
            let mut event_source = match EventSource::new("/api/studio/program/events") {
                Ok(source) => source,
                Err(error) => {
                    status.set(Some(format!("连接 Program 激活事件失败: {error}")));
                    return;
                }
            };
            let mut events = match event_source.subscribe("activated") {
                Ok(events) => events,
                Err(error) => {
                    status.set(Some(format!("订阅 Program 激活事件失败: {error}")));
                    return;
                }
            };
            while let Some(event) = events.next().await {
                match event {
                    Ok(_) => generation.with_mut(|value| *value = value.saturating_add(1)),
                    Err(error) => {
                        status.set(Some(format!("Program 激活事件连接已关闭: {error}")));
                        return;
                    }
                }
            }
        });
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn use_program_activation_refresh(_generation: Signal<u64>, _status: Signal<Option<String>>) {}

fn application_scenes(scenes: &[MenuDefinition]) -> Vec<ApplicationSceneItem> {
    scenes
        .iter()
        .filter(|scene| scene.enabled)
        .map(|scene| ApplicationSceneItem {
            id: scene.id.to_string(),
            label: scene.title.clone(),
        })
        .collect()
}

fn application_menus(scene: &MenuDefinition) -> Vec<ApplicationMenuItem> {
    if scene.page_id.is_some() {
        return vec![application_menu(scene)];
    }
    scene
        .children
        .iter()
        .filter(|menu| menu.enabled)
        .map(application_menu)
        .collect()
}

fn application_menu(menu: &MenuDefinition) -> ApplicationMenuItem {
    ApplicationMenuItem {
        id: menu.id.to_string(),
        label: menu.title.clone(),
        icon: menu.icon.clone(),
        page_id: menu.page_id.map(|page_id| page_id.to_string()),
        enabled: menu.enabled,
        children: menu.children.iter().map(application_menu).collect(),
    }
}

fn first_visible_page(menus: &[MenuDefinition]) -> Option<SymbolId> {
    menus
        .iter()
        .filter(|menu| menu.enabled)
        .find_map(|menu| menu.page_id.or_else(|| first_visible_page(&menu.children)))
}

fn first_visible_page_in_scene(scene: &MenuDefinition) -> Option<SymbolId> {
    scene
        .page_id
        .or_else(|| first_visible_page(&scene.children))
}

fn first_enabled_scene(menus: &[MenuDefinition]) -> Option<SymbolId> {
    menus.iter().find(|menu| menu.enabled).map(|menu| menu.id)
}

fn enabled_scene(menus: &[MenuDefinition], scene_id: SymbolId) -> Option<&MenuDefinition> {
    menus
        .iter()
        .find(|scene| scene.enabled && scene.id == scene_id)
}

fn scene_id_for_page(menus: &[MenuDefinition], page_id: SymbolId) -> Option<SymbolId> {
    menus
        .iter()
        .filter(|scene| scene.enabled)
        .find_map(|scene| menu_contains_page(scene, page_id).then_some(scene.id))
}

fn menu_contains_page(menu: &MenuDefinition, page_id: SymbolId) -> bool {
    menu.page_id == Some(page_id)
        || menu
            .children
            .iter()
            .filter(|child| child.enabled)
            .any(|child| menu_contains_page(child, page_id))
}

fn menu_id_for_page(menu: &MenuDefinition, page_id: SymbolId) -> Option<SymbolId> {
    if menu.page_id == Some(page_id) {
        return Some(menu.id);
    }
    menu.children
        .iter()
        .filter(|child| child.enabled)
        .find_map(|child| menu_id_for_page(child, page_id))
}

#[component]
pub fn EndpointPage(page_id: String) -> Element {
    let image = use_resource(move || async move {
        get_api::<ProgramImage>("", "/api/runtime/program/image").await
    });
    let Some(result) = image.read().as_ref().cloned() else {
        return state("正在加载接口页面", false);
    };
    let image = match result {
        Ok(image) => image,
        Err(error) => return state(&error, true),
    };
    let page_id = match SymbolId::parse(&page_id) {
        Ok(page_id) => page_id,
        Err(error) => return state(&error.to_string(), true),
    };
    let Some(page) = image.pages.get(&page_id).cloned() else {
        return state("接口页面编译产物不存在", true);
    };
    rsx! {
        EndpointWorkbench {
            context: ConventionPageContext {
                api_base_url: String::new(),
                route: String::new(),
                page,
            }
        }
    }
}

#[component]
pub fn RuntimePage(page_id: String) -> Element {
    let image = use_resource(move || async move {
        get_api::<ProgramImage>("", "/api/runtime/program/image").await
    });
    let Some(result) = image.read().as_ref().cloned() else {
        return state("正在加载页面", false);
    };
    let image = match result {
        Ok(image) => image,
        Err(error) => return state(&error, true),
    };
    let page_id = match SymbolId::parse(&page_id) {
        Ok(page_id) => page_id,
        Err(error) => return state(&error.to_string(), true),
    };
    let Some(page) = image.pages.get(&page_id).cloned() else {
        return state("页面编译产物不存在", true);
    };
    render_compiled_page(
        &image,
        page,
        ConventionPageRenderer::new(missing_convention_page),
    )
}

fn missing_convention_page(_page_id: SymbolId) -> Element {
    state("约定页面未生成", true)
}

fn render_compiled_page(
    image: &ProgramImage,
    page: CompiledPage,
    render_convention: ConventionPageRenderer,
) -> Element {
    match &page.renderer {
        CompiledPageRenderer::ConventionFile { .. } => render_convention.render(page.id),
        CompiledPageRenderer::MenuTree => rsx! {
            crate::ui::ProgramMenuTreePage {
                api_base_url: String::new(),
                title: page.title,
            }
        },
        CompiledPageRenderer::TreeTable { .. } | CompiledPageRenderer::CrudTable { .. } => rsx! {
            BuiltInPage {
                api_base_url: String::new(),
                image: image.clone(),
                row_actions: row_actions_for_page(&image.menus, page.id).unwrap_or_default(),
                page,
            }
        },
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

fn state(message: &str, error: bool) -> Element {
    rsx! {
        div {
            class: if error { "aio-runtime-error" } else { "aio-runtime-table-state" },
            role: if error { "alert" } else { "status" },
            "{message}"
        }
    }
}
