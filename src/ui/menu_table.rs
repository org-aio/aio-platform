use super::*;

#[derive(Clone)]
pub(super) struct MenuTableContext {
    pub(super) api_base_url: String,
    pub(super) program_id: String,
    pub(super) version: i64,
    pub(super) pages: Arc<Vec<PageDefinition>>,
    pub(super) routes: Arc<Vec<RouteDefinition>>,
    pub(super) permissions: Arc<Vec<PermissionDefinition>>,
    pub(super) generation: Signal<u64>,
    pub(super) status: Signal<Option<String>>,
    pub(super) editor_target: Signal<Option<MenuEditorTarget>>,
    pub(super) deleting_menu: Signal<Option<SymbolId>>,
    pub(super) collapsed_menus: Signal<BTreeSet<SymbolId>>,
}

pub(super) fn menu_table_cell(
    cell: DataTableCellContext<MenuTableRow>,
    context: MenuTableContext,
) -> Element {
    let menu = cell.row.menu;
    let menu_id = menu.id;
    let menu_name = menu.name.clone();
    let menu_title = menu.title.clone();
    let child_count = menu.children.len();
    let is_collapsed = (context.collapsed_menus)().contains(&menu_id);
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
        .collect::<Vec<_>>();
    let icon = resolved_navigation_icon(menu.icon.as_deref(), &menu.name);
    let mut toggle_context = context.clone();
    let mut edit_context = context.clone();
    let mut add_context = context.clone();
    let mut delete_context = context.clone();
    let enable_context = context.clone();
    let delete_kind = if cell.row.depth == 0 {
        "场景"
    } else {
        "菜单"
    };
    match cell.column.key.as_str() {
        "name" => rsx! {
            TreeIndent {
                depth: cell.row.depth,
                root: cell.row.depth == 0,
                    if child_count == 0 {
                    span { class: "aio-menu-cell__tree-spacer" }
                    } else {
                        Button {
                        class: if is_collapsed {
                            "aio-menu-cell__tree-toggle"
                        } else {
                            "aio-menu-cell__tree-toggle is-open"
                        },
                            r#type: "button",
                        size: ButtonSize::IconXs,
                        variant: ButtonVariant::Ghost,
                            title: if is_collapsed { "展开菜单" } else { "收起菜单" },
                        aria_label: if is_collapsed {
                            format!("展开菜单 {menu_name}")
                        } else {
                            format!("收起菜单 {menu_name}")
                        },
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            toggle_context.collapsed_menus.with_mut(|items| {
                                if !items.remove(&menu_id) {
                                    items.insert(menu_id);
                                }
                            });
                        },
                        icons::ChevronRight { class: "size-4" }
                        }
                    }
                div { class: "aio-menu-cell__identity",
                    strong { "{menu_title}" }
                    code { "{menu_name}" }
                }
            }
        },
        "icon" => rsx! {
            span { class: "aio-menu-cell__icon", title: "{icon}",
                NavigationIcon { name: icon.to_owned(), class: "size-4".to_owned() }
            }
        },
        "sort" => rsx! { "{cell.row.position + 1}" },
        "permissions" => rsx! {
            div { class: "aio-menu-cell__permissions",
                if permission_names.is_empty() {
                    span { class: "aio-menu-cell__empty", "无权限限制" }
                } else {
                    for permission_name in permission_names {
                        code { "{permission_name}" }
                    }
                }
            }
        },
        "route" => rsx! { code { class: "aio-menu-cell__code", "{route_path}" } },
        "page" => rsx! {
            if let Some(page) = page {
                div { class: "aio-menu-cell__page",
                    strong { "{page.title}" }
                    code { "{page.name}" }
                }
            } else {
                span { class: "aio-menu-cell__empty", "目录节点" }
            }
        },
        "enabled" => rsx! {
            div { class: "aio-menu-cell__enabled",
                Checkbox {
                    checked: Some(checkbox_state(menu.enabled)),
                    aria_label: if menu.enabled {
                        format!("停用菜单 {menu_name}")
                    } else {
                        format!("启用菜单 {menu_name}")
                    },
                        on_checked_change: move |checked| submit_patches(
                            enable_context.api_base_url.clone(),
                            enable_context.program_id.clone(),
                            enable_context.version,
                            vec![GraphPatch::SetProperty {
                                target_id: menu_id,
                                property: crate::EditableProperty::MenuEnabled,
                                value: serde_json::Value::Bool(checkbox_is_checked(checked)),
                            }],
                            enable_context.generation,
                            enable_context.status,
                        ),
                    }
                Badge { variant: BadgeVariant::Outline,
                    if menu.enabled { "启用" } else { "停用" }
                }
            }
        },
        "actions" => rsx! {
            div { class: "aio-menu-cell__actions",
                    Button {
                        r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "编辑{delete_kind} {menu_name}",
                    aria_label: "编辑{delete_kind} {menu_name}",
                    onclick: move |event: MouseEvent| {
                        event.stop_propagation();
                        edit_context.deleting_menu.set(None);
                        edit_context.editor_target.set(Some(MenuEditorTarget::Edit(menu_id)));
                    },
                    icons::Pencil { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "在 {menu_name} 下新建菜单",
                    aria_label: "在 {menu_name} 下新建菜单",
                    onclick: move |event: MouseEvent| {
                        event.stop_propagation();
                            add_context.collapsed_menus.with_mut(|items| {
                                items.remove(&menu_id);
                            });
                        add_context.deleting_menu.set(None);
                        add_context.editor_target.set(Some(MenuEditorTarget::Create {
                            menu_id: SymbolId::new(),
                            parent_id: menu_id,
                            index: child_count,
                        }));
                        },
                    icons::Plus { class: "size-4" }
                    }
                    Button {
                        r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                        title: "删除{delete_kind}",
                    aria_label: "删除{delete_kind} {menu_name}",
                    onclick: move |event: MouseEvent| {
                        event.stop_propagation();
                        delete_context.editor_target.set(None);
                        delete_context.deleting_menu.set(Some(menu_id));
                    },
                    icons::Trash2 { class: "size-4" }
                }
            }
        },
        _ => rsx! { "—" },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MenuParentOption {
    pub(super) id: SymbolId,
    pub(super) name: String,
    pub(super) title: String,
    pub(super) depth: usize,
}
