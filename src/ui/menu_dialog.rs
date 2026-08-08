use super::*;

#[component]
pub(super) fn MenuEditorDialog(
    menu_id: SymbolId,
    menu: Option<MenuDefinition>,
    mode: MenuEditorMode,
    selected_scene: MenuDefinition,
    root_menus: Vec<MenuDefinition>,
    pages: Vec<PageDefinition>,
    routes: Vec<RouteDefinition>,
    permissions: Vec<PermissionDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor_target: Signal<Option<MenuEditorTarget>>,
) -> Element {
    let editing = menu.is_some();
    let initial_menu = menu.unwrap_or_else(|| MenuDefinition {
        id: menu_id,
        name: String::new(),
        title: String::new(),
        state: DefinitionState::Known,
        icon: None,
        page_id: None,
        enabled: true,
        children: Vec::new(),
        required_permissions: Vec::new(),
        row_actions: crate::MenuRowActions::default(),
    });
    let current_page_id = initial_menu
        .page_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let route = initial_menu.page_id.and_then(|page_id| {
        routes
            .iter()
            .find(|route| route.page_id == page_id)
            .cloned()
    });
    let route_id = route.as_ref().map(|route| route.id);
    let route_path = route
        .as_ref()
        .map(|route| route.path.clone())
        .unwrap_or_default();
    let initial_parent_id = match mode {
        MenuEditorMode::Create { parent_id, .. } | MenuEditorMode::Edit { parent_id, .. } => {
            parent_id
        }
    };
    let initial_position = match mode {
        MenuEditorMode::Create { index, .. } => index,
        MenuEditorMode::Edit { position, .. } => position,
    };
    let root_scene = matches!(
        mode,
        MenuEditorMode::Edit {
            collection: ChildCollection::Menus,
            ..
        }
    );
    let parent_options = menu_parent_options(&selected_scene, editing.then_some(menu_id));
    let mut selected_parent = use_signal(move || initial_parent_id.to_string());
    let initial_selected_page = current_page_id.clone();
    let mut selected_page = use_signal(move || initial_selected_page);
    let selected_parent_id = SymbolId::parse(&selected_parent()).unwrap_or(initial_parent_id);
    let initial_sibling_count = match mode {
        MenuEditorMode::Create { index, .. } => index,
        MenuEditorMode::Edit { sibling_count, .. } => sibling_count,
    };
    let selected_parent_count = if editing && selected_parent_id == initial_parent_id {
        initial_sibling_count
    } else if root_scene {
        root_menus.len()
    } else {
        menu_child_count(&root_menus, selected_parent_id).unwrap_or_default()
    };
    let sort_max = if editing && selected_parent_id == initial_parent_id {
        selected_parent_count.max(1)
    } else {
        selected_parent_count.saturating_add(1).max(1)
    };
    let selected_permissions = initial_menu.required_permissions.clone();
    let detail_access = menu_action_value(&initial_menu.row_actions.detail);
    let edit_access = menu_action_value(&initial_menu.row_actions.edit);
    let delete_access = menu_action_value(&initial_menu.row_actions.delete);
    let initial_name = initial_menu.name.clone();
    let initial_title = initial_menu.title.clone();
    let initial_icon = initial_menu.icon.clone().unwrap_or_default();
    let initial_enabled = initial_menu.enabled;
    let submit_current_page_id = current_page_id.clone();
    let submit_permissions = permissions.clone();
    let close_editor = use_callback(move |_: ()| editor_target.set(None));

    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-menu-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    close_editor.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑菜单" } else { "新建菜单" } }
                    DialogDescription { "配置导航层级、页面入口和行操作授权" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭菜单编辑",
                    aria_label: "关闭菜单编辑",
                    onclick: move |_| close_editor.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let name = form_text(&event, "name").trim().to_owned();
                let title = form_text(&event, "title").trim().to_owned();
                if !page_identifier_is_valid(&name) {
                    status.set(Some("菜单标识必须以小写字母开头，只能包含小写字母、数字、下划线或连字符".to_owned()));
                    return;
                }
                if title.is_empty() {
                    status.set(Some("菜单名称不能为空".to_owned()));
                    return;
                }
                if menu_name_exists(&root_menus, &name, editing.then_some(menu_id)) {
                    status.set(Some(format!("菜单标识已存在: {name}")));
                    return;
                }
                let icon = form_text(&event, "icon").trim().to_owned();
                let page_id = selected_page();
                let path = form_text(&event, "path").trim().to_owned();
                if page_id == submit_current_page_id
                    && route_id.is_some()
                    && let Err(error) = validate_route_path(&path)
                {
                    status.set(Some(error.to_string()));
                    return;
                }
                let enabled = !form_text(&event, "menu_enabled").is_empty();
                let required_permissions = menu_permissions_from_form(&event, &submit_permissions);
                let row_actions = crate::MenuRowActions {
                    detail: menu_action_from_form(&event, "detail_access"),
                    edit: menu_action_from_form(&event, "edit_access"),
                    delete: menu_action_from_form(&event, "delete_access"),
                };
                let row_actions_value = match serde_json::to_value(row_actions.clone()) {
                    Ok(value) => value,
                    Err(error) => {
                        status.set(Some(format!("序列化行操作权限失败: {error}")));
                        return;
                    }
                };
                let requested_position = form_text(&event, "sort")
                    .parse::<usize>()
                    .unwrap_or(initial_position + 1)
                    .saturating_sub(1)
                    .min(sort_max.saturating_sub(1));
                let patches = match mode {
                    MenuEditorMode::Create { parent_id, .. } => {
                        let menu = MenuDefinition {
                            id: menu_id,
                            name,
                            title,
                            state: DefinitionState::Known,
                            icon: (!icon.is_empty()).then_some(icon),
                            page_id: SymbolId::parse(&page_id).ok(),
                            enabled,
                            children: Vec::new(),
                            required_permissions,
                            row_actions,
                        };
                        vec![GraphPatch::Insert {
                            parent_id,
                            collection: ChildCollection::MenuChildren,
                            index: requested_position,
                            entity: GraphEntity::Menu(menu),
                        }]
                    }
                    MenuEditorMode::Edit {
                        parent_id,
                        collection,
                        position,
                        ..
                    } => {
                        let mut patches = vec![
                            GraphPatch::Rename {
                                target_id: menu_id,
                                name,
                                title: Some(title),
                            },
                            GraphPatch::SetProperty {
                                target_id: menu_id,
                                property: crate::EditableProperty::Icon,
                                value: if icon.is_empty() {
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
                                property: crate::EditableProperty::MenuEnabled,
                                value: serde_json::Value::Bool(enabled),
                            },
                            GraphPatch::SetProperty {
                                target_id: menu_id,
                                property: crate::EditableProperty::MenuPermissions,
                                value: serde_json::json!(required_permissions),
                            },
                            GraphPatch::SetProperty {
                                target_id: menu_id,
                                property: crate::EditableProperty::MenuRowActions,
                                value: row_actions_value,
                            },
                        ];
                        if page_id == submit_current_page_id
                            && let Some(route_id) = route_id
                        {
                            patches.push(GraphPatch::SetProperty {
                                target_id: route_id,
                                property: crate::EditableProperty::RoutePath,
                                value: serde_json::Value::String(path),
                            });
                        }
                        let next_parent_id = if collection == ChildCollection::Menus {
                            parent_id
                        } else {
                            selected_parent_id
                        };
                        let next_collection = if collection == ChildCollection::Menus {
                            ChildCollection::Menus
                        } else {
                            ChildCollection::MenuChildren
                        };
                        if next_parent_id != parent_id || requested_position != position {
                            patches.push(GraphPatch::Move {
                                target_id: menu_id,
                                parent_id: next_parent_id,
                                collection: next_collection,
                                index: requested_position,
                            });
                        }
                        patches
                    }
                };
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    patches,
                    generation,
                    status,
                );
                close_editor.call(());
            },
                div { class: "aio-definition-dialog__grid aio-definition-dialog__grid--three",
                    label {
                        span { "菜单标识" }
                        Input {
                            class: "aio-input",
                            name: "name",
                            aria_label: "菜单标识",
                            placeholder: "例如 order-list",
                            value: initial_name,
                        }
                    }
                    label {
                        span { "菜单名称" }
                        Input {
                            class: "aio-input",
                            name: "title",
                            aria_label: "菜单名称",
                            placeholder: "例如 订单管理",
                            value: initial_title,
                        }
                    }
                    label {
                        span { "图标" }
                        Input {
                            class: "aio-input",
                            name: "icon",
                            aria_label: "菜单图标",
                            placeholder: "例如 package",
                            value: initial_icon,
                        }
                    }
                    label { class: "aio-definition-dialog__checkbox-field",
                        Checkbox {
                            name: "menu_enabled",
                            default_checked: checkbox_state(initial_enabled),
                            aria_label: "启用菜单",
                        }
                        span { "启用菜单" }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "层级与页面" }
                    div { class: "aio-definition-dialog__grid aio-definition-dialog__grid--three",
                        if editing && !root_scene {
                            label {
                                span { "父级菜单" }
                                select {
                                    class: "aio-input",
                                    aria_label: "父级菜单",
                                    value: selected_parent(),
                                    onchange: move |event: FormEvent| selected_parent.set(event.value()),
                                    for parent in &parent_options {
                                        option {
                                            value: "{parent.id}",
                                            selected: selected_parent() == parent.id.to_string(),
                                            {menu_parent_option_label(parent)}
                                        }
                                    }
                                }
                            }
                        }
                        label {
                            span { "排序" }
                            Input {
                                class: "aio-input",
                                r#type: "number",
                                name: "sort",
                                aria_label: "菜单排序",
                                min: "1",
                                max: "{sort_max}",
                                value: "{initial_position + 1}",
                            }
                        }
                        label {
                            span { "页面" }
                            select {
                                class: "aio-input",
                                aria_label: "菜单页面",
                                value: selected_page(),
                                onchange: move |event: FormEvent| selected_page.set(event.value()),
                                option { value: "", selected: selected_page().is_empty(), "无页面（目录）" }
                                for page in &pages {
                                    option {
                                        value: "{page.id}",
                                        selected: selected_page() == page.id.to_string(),
                                        "{page.title} · {page.name}"
                                    }
                                }
                            }
                        }
                        label {
                            span { "路由" }
                            Input {
                                class: "aio-input",
                                name: "path",
                                aria_label: "菜单路由",
                                value: route_path,
                                disabled: route_id.is_none() || selected_page() != current_page_id,
                                placeholder: "目录节点没有路由",
                            }
                        }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "菜单访问权限" }
                    if permissions.is_empty() {
                        p { class: "aio-definition-dialog__empty-state", "暂无权限定义" }
                    } else {
                        div { class: "aio-definition-dialog__choice-list",
                            for permission in &permissions {
                                label {
                                    Checkbox {
                                        name: "{menu_permission_input_name(permission.id)}",
                                        default_checked: checkbox_state(selected_permissions.contains(&permission.id)),
                                        aria_label: "菜单需要权限 {permission.title}",
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
                section { class: "aio-definition-dialog__section",
                    h3 { "行操作权限" }
                    div { class: "aio-definition-dialog__grid aio-definition-dialog__grid--three",
                        {menu_action_select("详情权限", "detail_access", &detail_access, &permissions)}
                        {menu_action_select("编辑权限", "edit_access", &edit_access, &permissions)}
                        {menu_action_select("删除权限", "delete_access", &delete_access, &permissions)}
                    }
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| close_editor.call(()),
                        "取消"
                    }
                    Button {
                        r#type: "submit",
                        icons::Save { class: "size-4" }
                        if editing { "保存菜单" } else { "创建菜单" }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn MenuDeleteDialog(
    row: MenuTableRow,
    menus: Vec<MenuDefinition>,
    routes: Vec<RouteDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut deleting_menu: Signal<Option<SymbolId>>,
) -> Element {
    let menu_id = row.menu.id;
    let menu_title = row.menu.title.clone();
    let delete_kind = if row.depth == 0 { "场景" } else { "菜单" };
    let descendant_count = menu_descendant_count(&row.menu).saturating_sub(1);
    let patches = delete_menu_patches(&menus, &routes, menu_id);
    let page_ids = menus
        .iter()
        .flat_map(|menu| menu_page_ids(menu))
        .collect::<BTreeSet<_>>();
    let deleted_page_count = patches
        .iter()
        .filter(|patch| matches!(patch, GraphPatch::Delete { target_id } if page_ids.contains(target_id)))
        .count();
    let route_ids = routes.iter().map(|route| route.id).collect::<BTreeSet<_>>();
    let deleted_route_count = patches
        .iter()
        .filter(|patch| matches!(patch, GraphPatch::Delete { target_id } if route_ids.contains(target_id)))
        .count();
    rsx! {
        Dialog {
            class: "aio-endpoint-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting_menu.set(None);
                }
            },
            DialogTitle { "删除{delete_kind}" }
            DialogDescription {
                "确认删除“{menu_title}”？将同时删除 {descendant_count} 个子菜单、{deleted_page_count} 个独占页面和 {deleted_route_count} 条独占路由，此操作不可恢复。"
            }
            footer { class: "aio-endpoint-confirm-dialog__actions",
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting_menu.set(None),
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
                        deleting_menu.set(None);
                    },
                    "删除"
                }
            }
        }
    }
}

pub(super) fn menu_name_exists(
    menus: &[MenuDefinition],
    name: &str,
    excluded_id: Option<SymbolId>,
) -> bool {
    menus.iter().any(|menu| {
        (Some(menu.id) != excluded_id && menu.name == name)
            || menu_name_exists(&menu.children, name, excluded_id)
    })
}

pub(super) fn menu_parent_options(
    scene: &MenuDefinition,
    excluded_menu_id: Option<SymbolId>,
) -> Vec<MenuParentOption> {
    let mut excluded_ids = BTreeSet::new();
    if let Some(excluded_menu_id) = excluded_menu_id
        && let Some(menu) = find_menu_definition(scene, excluded_menu_id)
    {
        collect_menu_ids(menu, &mut excluded_ids);
    }
    let mut options = Vec::new();
    collect_menu_parent_options(scene, 0, &excluded_ids, &mut options);
    options
}

pub(super) fn menu_parent_option_label(option: &MenuParentOption) -> String {
    format!(
        "{}{} · {}",
        "  ".repeat(option.depth),
        option.title,
        option.name
    )
}

pub(super) fn find_menu_definition(
    menu: &MenuDefinition,
    target_id: SymbolId,
) -> Option<&MenuDefinition> {
    if menu.id == target_id {
        return Some(menu);
    }
    menu.children
        .iter()
        .find_map(|child| find_menu_definition(child, target_id))
}

pub(super) fn collect_menu_ids(menu: &MenuDefinition, ids: &mut BTreeSet<SymbolId>) {
    ids.insert(menu.id);
    for child in &menu.children {
        collect_menu_ids(child, ids);
    }
}

pub(super) fn collect_menu_parent_options(
    menu: &MenuDefinition,
    depth: usize,
    excluded_ids: &BTreeSet<SymbolId>,
    options: &mut Vec<MenuParentOption>,
) {
    if excluded_ids.contains(&menu.id) {
        return;
    }
    options.push(MenuParentOption {
        id: menu.id,
        name: menu.name.clone(),
        title: menu.title.clone(),
        depth,
    });
    for child in &menu.children {
        collect_menu_parent_options(child, depth + 1, excluded_ids, options);
    }
}

pub(super) fn menu_descendant_count(menu: &MenuDefinition) -> usize {
    1 + menu
        .children
        .iter()
        .map(menu_descendant_count)
        .sum::<usize>()
}

pub(super) fn menu_page_ids(menu: &MenuDefinition) -> Vec<SymbolId> {
    menu.page_id
        .into_iter()
        .chain(menu.children.iter().flat_map(menu_page_ids))
        .collect()
}

pub(super) fn menu_permission_input_name(permission_id: SymbolId) -> String {
    format!("menu_permission_{permission_id}")
}

pub(super) fn menu_permissions_from_form(
    event: &FormEvent,
    permissions: &[PermissionDefinition],
) -> Vec<SymbolId> {
    permissions
        .iter()
        .filter(|permission| {
            !form_text(event, &menu_permission_input_name(permission.id)).is_empty()
        })
        .map(|permission| permission.id)
        .collect()
}

pub(super) fn menu_action_value(access: &crate::MenuActionAccess) -> String {
    match access {
        crate::MenuActionAccess::Hidden => "hidden".to_owned(),
        crate::MenuActionAccess::Public => "public".to_owned(),
        crate::MenuActionAccess::Permission { permission_id } => {
            format!("permission:{permission_id}")
        }
    }
}

pub(super) fn menu_action_from_form(event: &FormEvent, name: &str) -> crate::MenuActionAccess {
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

pub(super) fn menu_action_select(
    title: &'static str,
    name: &'static str,
    selected: &str,
    permissions: &[PermissionDefinition],
) -> Element {
    rsx! {
        label {
            span { "{title}" }
            select { name, class: "aio-input", aria_label: "{title}",
                option { value: "hidden", selected: selected == "hidden", "不显示" }
                option { value: "public", selected: selected == "public", "公开" }
                for permission in permissions {
                    option {
                        value: "permission:{permission.id}",
                        selected: selected == format!("permission:{}", permission.id),
                        "{permission.name} · {permission.title}"
                    }
                }
            }
        }
    }
}
