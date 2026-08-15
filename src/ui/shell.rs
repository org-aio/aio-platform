use super::*;

#[component]
pub fn StudioPage(api_base_url: String, published_scene: Option<SymbolId>) -> Element {
    let draft_generation = use_signal(|| 0_u64);
    let mut studio_tab = use_signal(StudioTab::default);
    let mut selected_draft_scene = use_signal(move || published_scene);
    let status = use_signal(|| None::<String>);
    let selected_model = use_signal(|| None::<SymbolId>);
    let selected_page = use_signal(|| None::<SymbolId>);
    let selected_function = use_signal(|| None::<SymbolId>);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = draft_generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    use_effect(move || {
        if let Some(Ok(draft)) = draft.read().as_ref() {
            let scene_ids = draft
                .definition
                .menus
                .iter()
                .map(|scene| scene.id)
                .collect::<Vec<_>>();
            let next =
                preferred_draft_scene_id(&scene_ids, selected_draft_scene(), published_scene);
            if selected_draft_scene() != next {
                selected_draft_scene.set(next);
            }
        }
    });

    let draft_snapshot = draft.read().as_ref().cloned();
    let models_panel_key = "studio-models";

    rsx! {
        section { class: "aio-studio-shell min-h-[calc(100vh-8rem)] border bg-background",
            header { class: "aio-studio-shell__toolbar border-b px-3",
                nav { class: "aio-studio-view-tabs", aria_label: "Studio 管理视图",
                    Button {
                        class: if studio_tab() == StudioTab::Applications { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "应用生成",
                        onclick: move |_| studio_tab.set(StudioTab::Applications),
                        "应用"
                    }
                    Button {
                        class: if studio_tab() == StudioTab::Models { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "模型定义",
                        onclick: move |_| studio_tab.set(StudioTab::Models),
                        "模型"
                    }
                    Button {
                        class: if studio_tab() == StudioTab::Pages { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "页面定义",
                        onclick: move |_| studio_tab.set(StudioTab::Pages),
                        "页面"
                    }
                    Button {
                        class: if studio_tab() == StudioTab::Functions { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "函数定义",
                        onclick: move |_| studio_tab.set(StudioTab::Functions),
                        "函数"
                    }
                    Button {
                        class: if studio_tab() == StudioTab::Endpoints { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "接口定义",
                        onclick: move |_| studio_tab.set(StudioTab::Endpoints),
                        "接口"
                    }
                    Button {
                        class: if studio_tab() == StudioTab::Menus { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "菜单管理",
                        onclick: move |_| studio_tab.set(StudioTab::Menus),
                        "菜单"
                    }
                    Button {
                        class: if studio_tab() == StudioTab::Permissions { "is-active" } else { "" },
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        aria_label: "权限定义",
                        onclick: move |_| studio_tab.set(StudioTab::Permissions),
                        "权限"
                    }
                }
                if let Some(message) = status() {
                    Badge { variant: BadgeVariant::Outline, "{message}" }
                }
            }
            main { class: "aio-studio-shell__content min-w-0 p-4",
                match draft_snapshot {
                    Some(Ok(draft)) => match studio_tab() {
                        StudioTab::Applications => rsx! {
                            ApplicationPanel {
                                key: "studio-application:{draft.version}",
                                draft,
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                            }
                        },
                        StudioTab::Models => rsx! {
                            ModelsPanel {
                                key: "{models_panel_key}",
                                draft,
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                                selected_model,
                                preferred_model_id: None,
                            }
                        },
                        StudioTab::Pages => rsx! {
                            PagesPanel {
                                key: "studio-pages:{draft.version}",
                                draft,
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                                selected_page,
                            }
                        },
                        StudioTab::Functions => rsx! {
                            FunctionsPanel {
                                key: "studio-functions:{draft.version}",
                                draft,
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                                selected_function,
                            }
                        },
                        StudioTab::Endpoints => rsx! {
                            EndpointCatalogPanel {
                                key: "studio-endpoints:{draft.version}",
                                draft,
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                                selected_page,
                            }
                        },
                        StudioTab::Menus => rsx! {
                            MenusPanel {
                                draft,
                                selected_scene: selected_draft_scene(),
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                            }
                        },
                        StudioTab::Permissions => rsx! {
                            PermissionsPanel {
                                key: "studio-permissions:{draft.version}",
                                draft,
                                api_base_url: api_base_url.clone(),
                                generation: draft_generation,
                                status,
                            }
                        },
                    },
                    Some(Err(error)) => empty_panel(&error),
                    None => empty_panel("正在加载 Draft"),
                }
            }
        }
    }
}

/// 在发布应用内维护同一份 ProgramDefinition 菜单树。
#[component]
pub(crate) fn ProgramMenuTreePage(api_base_url: String, title: String) -> Element {
    let mut generation = use_signal(|| 0_u64);
    let status = use_signal(|| None::<String>);
    let mut selected_scene = use_signal(|| None::<SymbolId>);
    let pending_scene = use_signal(|| None::<SymbolId>);
    let mut scene_creator_open = use_signal(|| false);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    use_effect(move || {
        if let Some(Ok(draft)) = draft.read().as_ref() {
            let scene_ids = draft
                .definition
                .menus
                .iter()
                .map(|scene| scene.id)
                .collect::<Vec<_>>();
            let next = preferred_draft_scene_id(&scene_ids, selected_scene(), pending_scene());
            if selected_scene() != next {
                selected_scene.set(next);
            }
        }
    });
    let draft_snapshot = draft.read().as_ref().cloned();
    let scenes = draft_snapshot
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|draft| draft.definition.menus.clone())
        .unwrap_or_default();

    rsx! {
        section { class: "aio-program-menu-page",
            header { class: "aio-program-menu-page__header",
                div {
                    h2 { "{title}" }
                    p { "ProgramDefinition · {scenes.len()} 个场景" }
                }
                div { class: "aio-program-menu-page__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Outline,
                        title: "刷新菜单",
                        aria_label: "刷新菜单",
                        onclick: move |_| {
                            generation.with_mut(|value| *value = value.saturating_add(1));
                        },
                        icons::RefreshCw { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                        onclick: move |_| scene_creator_open.set(true),
                        icons::Plus { class: "size-4" }
                        "新建场景"
                    }
                }
            }
            if let Some(message) = status() {
                div { class: "aio-program-menu-page__notice", role: "status", "{message}" }
            }
            nav { class: "aio-program-menu-page__scenes", aria_label: "菜单场景",
                for scene in &scenes {
                    Button {
                        key: "{scene.id}",
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: if selected_scene() == Some(scene.id) {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        },
                        onclick: {
                            let scene_id = scene.id;
                            move |_| selected_scene.set(Some(scene_id))
                        },
                        "{scene.title}"
                    }
                }
            }
            main { class: "aio-program-menu-page__content",
                match draft_snapshot {
                    Some(Ok(draft)) => rsx! {
                        MenusPanel {
                            key: "runtime-menus:{draft.version}",
                            draft,
                            selected_scene: selected_scene(),
                            api_base_url: api_base_url.clone(),
                            generation,
                            status,
                        }
                    },
                    Some(Err(error)) => empty_panel(&error),
                    None => empty_panel("正在加载菜单定义"),
                }
            }
            if scene_creator_open() {
                AdminSceneCreator {
                    api_base_url,
                    pending_scene,
                    creator_open: scene_creator_open,
                    shared_generation: Some(generation),
                    shared_status: Some(status),
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
    menu_id: Option<SymbolId>,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    settings_open: Signal<bool>,
) -> Element {
    let settings_tab = use_signal(PageSettingsTab::default);
    let mut deleting_menu = use_signal(|| None::<SymbolId>);
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
    let menu_row = menu_id.and_then(|menu_id| {
        find_menu_table_row(&draft.definition.menus, draft.definition.id, menu_id)
    });
    let delete_action = menu_row.as_ref().map(|row| {
        let menu_id = row.menu.id;
        Callback::new(move |()| deleting_menu.set(Some(menu_id)))
    });
    rsx! {
        PageRendererSettings {
            key: "admin:{page_id}:{draft.version}",
            page,
            models: draft.definition.models.clone(),
            api_base_url: api_base_url.clone(),
            program_id: draft.program_id.clone(),
            version: draft.version,
            generation,
            status,
            settings_tab,
            settings_open,
            on_delete_menu: delete_action,
            draft: draft.clone(),
        }
        if deleting_menu().is_some()
            && let Some(row) = menu_row
        {
            MenuDeleteDialog {
                row,
                menus: draft.definition.menus.clone(),
                routes: draft.definition.routes.clone(),
                api_base_url,
                program_id: draft.program_id,
                version: draft.version,
                generation,
                status,
                deleting_menu,
                on_deleted: move |_| settings_open.set(false),
            }
        }
    }
}

/// 管理模式从左侧栏直接向当前场景新增菜单和页面。
#[component]
pub(crate) fn AdminMenuCreator(
    api_base_url: String,
    scene_id: SymbolId,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    creator_open: Signal<bool>,
) -> Element {
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
    let submit_api = api_base_url;
    let submit_program = draft.program_id.clone();
    rsx! {
        Dialog {
            class: "aio-definition-dialog",
            open: true,
            on_open_change: move |open| creator_open.set(open),
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "新建菜单" }
                    DialogDescription { "同时创建页面、路由和当前场景下的菜单入口" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭新建菜单",
                    aria_label: "关闭新建菜单",
                    onclick: move |_| creator_open.set(false),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let title = form_text(&event, "title").trim().to_owned();
                let path = form_text(&event, "path").trim().to_owned();
                if title.is_empty() || !path.starts_with('/') {
                    status.set(Some("页面标题不能为空，路由必须以 / 开头".to_owned()));
                    return;
                }
                let name = identifier_from_title(&title);
                if name.is_empty() {
                    status.set(Some("页面标题无法生成有效标识，请包含中文、字母或数字".to_owned()));
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
                label { r#for: "admin-page-title", "页面标题" }
                Input {
                    id: "admin-page-title",
                    name: "title",
                    class: "aio-input",
                    placeholder: "例如 订单管理"
                }
                label { r#for: "admin-page-path", "路由" }
                Input {
                    id: "admin-page-path",
                    name: "path",
                    class: "aio-input",
                    placeholder: "/orders"
                }
                if let Some(message) = status() {
                    p { class: "text-xs text-destructive", "{message}" }
                }
                footer {
                    Button { r#type: "submit", "添加" }
                    Button {
                        r#type: "button",
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
    #[props(default)] shared_generation: Option<Signal<u64>>,
    #[props(default)] shared_status: Option<Signal<Option<String>>>,
) -> Element {
    let local_generation = use_signal(|| 0_u64);
    let local_status = use_signal(|| None::<String>);
    let generation = shared_generation.unwrap_or(local_generation);
    let mut status = shared_status.unwrap_or(local_status);
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
        Dialog {
            class: "aio-definition-dialog",
            open: true,
            on_open_change: move |open| creator_open.set(open),
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "新建场景" }
                    DialogDescription { "创建顶栏场景，并在其中继续添加菜单" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭新建场景",
                    aria_label: "关闭新建场景",
                    onclick: move |_| creator_open.set(false),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let title = form_text(&event, "title").trim().to_owned();
                if title.is_empty() {
                    status.set(Some("场景标题不能为空".to_owned()));
                    return;
                }
                let name = identifier_from_title(&title);
                if name.is_empty() {
                    status.set(Some("场景标题无法生成有效标识，请包含中文、字母或数字".to_owned()));
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
                label { r#for: "admin-scene-title", "场景标题" }
                Input {
                    id: "admin-scene-title",
                    name: "title",
                    class: "aio-input",
                    placeholder: "例如 运维中心"
                }
                if let Some(message) = status() {
                    p { class: "text-xs text-destructive", "{message}" }
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| creator_open.set(false),
                        "取消"
                    }
                    Button { r#type: "submit", "创建场景" }
                }
            }
        }
    }
}
