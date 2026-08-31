use super::*;

#[component]
pub(super) fn PageNavigationSettings(
    menu: MenuDefinition,
    route: Option<RouteDefinition>,
    permissions: Vec<PermissionDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut settings_open: Signal<bool>,
) -> Element {
    let menu_id = menu.id;
    let menu_name = menu.name.clone();
    let initial_icon = resolved_navigation_icon(menu.icon.as_deref(), &menu.name).to_owned();
    let mut selected_icon = use_signal(move || initial_icon);
    let initial_detail_access = menu_action_value(&menu.row_actions.detail);
    let initial_edit_access = menu_action_value(&menu.row_actions.edit);
    let initial_delete_access = menu_action_value(&menu.row_actions.delete);
    let detail_access = use_signal(move || initial_detail_access);
    let edit_access = use_signal(move || initial_edit_access);
    let delete_access = use_signal(move || initial_delete_access);
    let selected_permissions = menu.required_permissions.clone();
    let initial_enabled = menu.enabled;
    let route_id = route.as_ref().map(|route| route.id);
    let route_path = route
        .as_ref()
        .map(|route| route.path.clone())
        .unwrap_or_default();
    let submit_permissions = permissions.clone();

    rsx! {
        form { class: "aio-page-settings__form aio-page-layout-form", onsubmit: move |event| {
            event.prevent_default();
            let title = form_text(&event, "title").trim().to_owned();
            if title.is_empty() {
                status.set(Some("导航名称不能为空".to_owned()));
                return;
            }
            let icon = form_text(&event, "icon").trim().to_owned();
            let enabled = !form_text(&event, "menu_enabled").is_empty();
            let path = form_text(&event, "path").trim().to_owned();
            if route_id.is_some()
                && let Err(error) = validate_route_path(&path)
            {
                status.set(Some(error.to_string()));
                return;
            }
            let required_permissions = menu_permissions_from_form(&event, &submit_permissions);
            let row_actions = crate::MenuRowActions {
                detail: menu_action_from_form(&event, "detail_access"),
                edit: menu_action_from_form(&event, "edit_access"),
                delete: menu_action_from_form(&event, "delete_access"),
            };
            let row_actions_value = match serde_json::to_value(row_actions) {
                Ok(value) => value,
                Err(error) => {
                    status.set(Some(format!("序列化行操作权限失败: {error}")));
                    return;
                }
            };
            let mut patches = vec![
                GraphPatch::Rename {
                    target_id: menu_id,
                    name: menu_name.clone(),
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
            if let Some(route_id) = route_id {
                patches.push(GraphPatch::SetProperty {
                    target_id: route_id,
                    property: crate::EditableProperty::RoutePath,
                    value: serde_json::Value::String(path),
                });
            }
            submit_patches(
                api_base_url.clone(),
                program_id.clone(),
                version,
                patches,
                generation,
                status,
            );
        },
            div { class: "aio-page-layout-form__content",
                div { class: "aio-page-layout-form__workspace",
                    section { class: "aio-page-layout-form__section",
                        header {
                            h2 { "导航配置" }
                            code { "{menu.name}" }
                        }
                        div { class: "aio-page-layout-form__fields",
                            label { r#for: "page-navigation-title", "导航名称" }
                            Input {
                                id: "page-navigation-title",
                                class: "aio-input",
                                name: "title",
                                aria_label: "导航名称",
                                value: menu.title.clone(),
                            }
                            label { "图标" }
                            div {
                                NavigationIconPicker {
                                    name: "icon",
                                    aria_label: "导航图标",
                                    value: selected_icon,
                                    on_value_change: move |value: String| selected_icon.set(value),
                                }
                            }
                            label { "状态" }
                            label { class: "aio-definition-dialog__checkbox-field",
                                Checkbox {
                                    name: "menu_enabled",
                                    default_checked: checkbox_state(initial_enabled),
                                    aria_label: "启用导航",
                                }
                                span { "启用导航" }
                            }
                            if route_id.is_some() {
                                label { r#for: "page-navigation-path", "页面路由" }
                                Input {
                                    id: "page-navigation-path",
                                    class: "aio-input",
                                    name: "path",
                                    aria_label: "页面路由",
                                    value: route_path,
                                }
                            }
                            label { "访问权限" }
                            if permissions.is_empty() {
                                p { class: "aio-definition-dialog__empty-state", "暂无权限定义" }
                            } else {
                                div { class: "aio-definition-dialog__choice-list",
                                    for permission in &permissions {
                                        label {
                                            Checkbox {
                                                name: "{menu_permission_input_name(permission.id)}",
                                                default_checked: checkbox_state(
                                                    selected_permissions.contains(&permission.id)
                                                ),
                                                aria_label: "导航需要权限 {permission.title}",
                                            }
                                            span {
                                                strong { "{permission.title}" }
                                                code { "{permission.name}" }
                                            }
                                        }
                                    }
                                }
                            }
                            label { "行操作权限" }
                            div { class: "aio-definition-dialog__grid aio-definition-dialog__grid--three",
                                {menu_action_select("详情权限", "detail_access", detail_access, &permissions)}
                                {menu_action_select("编辑权限", "edit_access", edit_access, &permissions)}
                                {menu_action_select("删除权限", "delete_access", delete_access, &permissions)}
                            }
                        }
                    }
                    aside { class: "aio-page-layout-summary", aria_label: "导航摘要",
                        header {
                            h2 { "导航摘要" }
                            Badge {
                                variant: if menu.enabled {
                                    BadgeVariant::Secondary
                                } else {
                                    BadgeVariant::Outline
                                },
                                if menu.enabled { "已启用" } else { "已停用" }
                            }
                        }
                        dl {
                            div { dt { "菜单标识" } dd { code { "{menu.name}" } } }
                            div { dt { "页面挂载" } dd { "当前页面" } }
                            div {
                                dt { "访问权限" }
                                dd { "{menu.required_permissions.len()} 项" }
                            }
                            div {
                                dt { "页面路由" }
                                dd {
                                    {route.as_ref().map(|route| route.path.as_str()).unwrap_or("无")}
                                }
                            }
                        }
                    }
                }
            }
            footer {
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| settings_open.set(false),
                    "取消"
                }
                Button {
                    r#type: "submit",
                    icons::Save { class: "size-4" }
                    "保存导航"
                }
            }
        }
    }
}

pub(super) fn unique_menu_for_page(
    menus: &[MenuDefinition],
    page_id: SymbolId,
) -> Option<MenuDefinition> {
    let mut matches = Vec::new();
    collect_menus_for_page(menus, page_id, &mut matches);
    (matches.len() == 1).then(|| matches.remove(0))
}

fn collect_menus_for_page(
    menus: &[MenuDefinition],
    page_id: SymbolId,
    matches: &mut Vec<MenuDefinition>,
) {
    for menu in menus {
        if menu.page_id == Some(page_id) {
            matches.push(menu.clone());
        }
        collect_menus_for_page(&menu.children, page_id, matches);
    }
}
