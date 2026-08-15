use super::*;

pub(super) fn menu_child_count(menus: &[MenuDefinition], target: SymbolId) -> Option<usize> {
    menus.iter().find_map(|menu| {
        if menu.id == target {
            return Some(menu.children.len());
        }
        menu_child_count(&menu.children, target)
    })
}

#[component]
pub(super) fn MenusPanel(
    draft: DraftSnapshot,
    selected_scene: Option<SymbolId>,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let collapsed_menus = use_signal(BTreeSet::<SymbolId>::new);
    let mut editor_target = use_signal(|| None::<MenuEditorTarget>);
    let deleting_menu = use_signal(|| None::<SymbolId>);
    let root_id = draft.definition.id;
    let selected = selected_scene.and_then(|scene_id| {
        draft
            .definition
            .menus
            .iter()
            .cloned()
            .enumerate()
            .find(|(_, scene)| scene.id == scene_id)
    });
    let all_rows = selected
        .as_ref()
        .map_or_else(Vec::new, |(position, scene)| {
            menu_table_rows(
                scene,
                0,
                *position,
                root_id,
                ChildCollection::Menus,
                draft.definition.menus.len(),
                &BTreeSet::new(),
            )
        });
    let visible_rows = selected
        .as_ref()
        .map_or_else(Vec::new, |(position, scene)| {
            menu_table_rows(
                scene,
                0,
                *position,
                root_id,
                ChildCollection::Menus,
                draft.definition.menus.len(),
                &collapsed_menus(),
            )
        });
    let table_context = MenuTableContext {
        api_base_url: api_base_url.clone(),
        program_id: draft.program_id.clone(),
        version: draft.version,
        pages: Arc::new(draft.definition.pages.clone()),
        routes: Arc::new(draft.definition.routes.clone()),
        permissions: Arc::new(draft.definition.permissions.clone()),
        generation,
        status,
        editor_target,
        deleting_menu,
        collapsed_menus,
    };
    let editing_row = editor_target().and_then(|target| match target {
        MenuEditorTarget::Edit(menu_id) => {
            all_rows.iter().find(|row| row.menu.id == menu_id).cloned()
        }
        MenuEditorTarget::Create { .. } => None,
    });
    let deleting_row = deleting_menu()
        .and_then(|menu_id| all_rows.iter().find(|row| row.menu.id == menu_id).cloned());
    let selected_scene_definition = selected.as_ref().map(|(_, scene)| scene.clone());
    let selected_scene_id = selected_scene_definition.as_ref().map(|scene| scene.id);
    let selected_scene_title = selected_scene_definition
        .as_ref()
        .map(|scene| scene.title.as_str())
        .unwrap_or("未选择场景");
    let selected_scene_child_count = selected_scene_definition
        .as_ref()
        .map(|scene| scene.children.len())
        .unwrap_or_default();
    let creating_target = editor_target().and_then(|target| match target {
        MenuEditorTarget::Create {
            menu_id,
            parent_id,
            index,
        } => Some((menu_id, parent_id, index)),
        MenuEditorTarget::Edit(_) => None,
    });
    rsx! {
        section { class: "aio-menu-management",
            header { class: "aio-menu-management__toolbar",
                div {
                    h2 { "菜单" }
                    p { "{selected_scene_title} · 场景与页面导航结构" }
                }
                div { class: "aio-menu-management__actions",
                    Badge { variant: BadgeVariant::Outline, "{visible_rows.len()} 项" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        disabled: selected_scene_id.is_none(),
                        onclick: move |_| {
                            let Some(parent_id) = selected_scene_id else {
                                return;
                            };
                            editor_target.set(Some(MenuEditorTarget::Create {
                                menu_id: SymbolId::new(),
                                parent_id,
                                index: selected_scene_child_count,
                            }));
                        },
                        icons::Plus { class: "size-4" }
                        "新建菜单"
                    }
                }
            }
            DataTable::<MenuTableRow> {
                class: "aio-menu-data-table",
                aria_label: "场景与菜单",
                rows: visible_rows,
                columns: menu_table_columns(),
                max_height: "40rem",
                empty_text: "暂无场景".to_owned(),
                row_key: |row: MenuTableRow| row.menu.id.to_string(),
                render_cell: move |cell: DataTableCellContext<MenuTableRow>| {
                    menu_table_cell(cell, table_context.clone())
                },
            }
            if let Some((menu_id, parent_id, index)) = creating_target
                && let Some(scene) = selected_scene_definition.clone()
            {
                MenuEditorDialog {
                    key: "create:{menu_id}",
                    menu_id,
                    menu: None,
                    mode: MenuEditorMode::Create { parent_id, index },
                    selected_scene: scene,
                    root_menus: draft.definition.menus.clone(),
                    pages: draft.definition.pages.clone(),
                    routes: draft.definition.routes.clone(),
                    permissions: draft.definition.permissions.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    editor_target,
                }
            } else if let Some(row) = editing_row
                && let Some(scene) = selected_scene_definition.clone()
            {
                MenuEditorDialog {
                    key: "edit:{row.menu.id}",
                    menu_id: row.menu.id,
                    menu: Some(row.menu.clone()),
                    mode: MenuEditorMode::Edit {
                        parent_id: row.parent_id,
                        collection: row.collection,
                        position: row.position,
                        sibling_count: row.sibling_count,
                    },
                    selected_scene: scene,
                    root_menus: draft.definition.menus.clone(),
                    pages: draft.definition.pages.clone(),
                    routes: draft.definition.routes.clone(),
                    permissions: draft.definition.permissions.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    editor_target,
                }
            }
            if let Some(row) = deleting_row {
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
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuEditorTarget {
    Create {
        menu_id: SymbolId,
        parent_id: SymbolId,
        index: usize,
    },
    Edit(SymbolId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuEditorMode {
    Create {
        parent_id: SymbolId,
        index: usize,
    },
    Edit {
        parent_id: SymbolId,
        collection: ChildCollection,
        position: usize,
        sibling_count: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MenuTableRow {
    pub(super) menu: MenuDefinition,
    pub(super) depth: usize,
    pub(super) position: usize,
    pub(super) parent_id: SymbolId,
    pub(super) collection: ChildCollection,
    pub(super) sibling_count: usize,
}

pub(super) fn menu_table_rows(
    menu: &MenuDefinition,
    depth: usize,
    position: usize,
    parent_id: SymbolId,
    collection: ChildCollection,
    sibling_count: usize,
    collapsed_menus: &BTreeSet<SymbolId>,
) -> Vec<MenuTableRow> {
    let mut rows = vec![MenuTableRow {
        menu: menu.clone(),
        depth,
        position,
        parent_id,
        collection,
        sibling_count,
    }];
    if collapsed_menus.contains(&menu.id) {
        return rows;
    }
    let child_count = menu.children.len();
    for (child_position, child) in menu.children.iter().enumerate() {
        rows.extend(menu_table_rows(
            child,
            depth + 1,
            child_position,
            menu.id,
            ChildCollection::MenuChildren,
            child_count,
            collapsed_menus,
        ));
    }
    rows
}

pub(super) fn find_menu_table_row(
    menus: &[MenuDefinition],
    program_id: SymbolId,
    target_id: SymbolId,
) -> Option<MenuTableRow> {
    menus
        .iter()
        .enumerate()
        .flat_map(|(position, scene)| {
            menu_table_rows(
                scene,
                0,
                position,
                program_id,
                ChildCollection::Menus,
                menus.len(),
                &BTreeSet::new(),
            )
        })
        .find(|row| row.menu.id == target_id)
}

pub(super) fn menu_table_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("name", "菜单名称")
            .width(300)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("icon", "图标")
            .width(72)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("sort", "排序")
            .width(72)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("permissions", "权限标识").width(260),
        DataTableColumn::leaf("route", "路由").width(260),
        DataTableColumn::leaf("page", "页面").width(190),
        DataTableColumn::leaf("enabled", "状态")
            .width(96)
            .align(DataTableAlign::Center)
            .fixed(DataTableFixed::Right),
        DataTableColumn::leaf("actions", "操作")
            .width(144)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}
