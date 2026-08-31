use super::*;

#[component]
pub(super) fn PermissionsPanel(
    draft: DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let mut permission_search = use_signal(String::new);
    let mut creating_permission = use_signal(|| None::<PermissionDefinition>);
    let mut editing_permission = use_signal(|| None::<SymbolId>);
    let mut deleting_permission = use_signal(|| None::<SymbolId>);
    let permission_count = draft.definition.permissions.len();
    let usage_counts = permission_usage_map(&draft.definition);
    let normalized_search = permission_search().trim().to_lowercase();
    let rows = draft
        .definition
        .permissions
        .iter()
        .filter(|permission| {
            definition_matches_search(&permission.name, &permission.title, &normalized_search)
        })
        .map(|permission| PermissionTableRow {
            permission: permission.clone(),
            usage_count: usage_counts
                .get(&permission.id)
                .copied()
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    let editing_dialog_permission = editing_permission().and_then(|permission_id| {
        draft
            .definition
            .permissions
            .iter()
            .find(|permission| permission.id == permission_id)
            .cloned()
    });
    let deleting_dialog_permission = deleting_permission().and_then(|permission_id| {
        draft
            .definition
            .permissions
            .iter()
            .find(|permission| permission.id == permission_id)
            .cloned()
            .map(|permission| PermissionTableRow {
                usage_count: usage_counts
                    .get(&permission.id)
                    .copied()
                    .unwrap_or_default(),
                permission,
            })
    });
    let creating_dialog_permission = creating_permission();
    let permissions = draft.definition.permissions.clone();
    let root_id = draft.definition.id;
    let program_id = draft.program_id.clone();
    let version = draft.version;
    rsx! {
        section { class: "aio-permission-management",
            header { class: "aio-permission-management__toolbar",
                div {
                    h2 { "权限定义" }
                    p { "权限目录供菜单、行操作、路由和函数复用" }
                }
                div { class: "aio-permission-management__actions",
                    div { class: "aio-permission-management__search",
                        icons::Search { class: "size-4" }
                        Input {
                            class: "aio-input",
                            aria_label: "搜索权限",
                            placeholder: "搜索权限",
                            value: permission_search(),
                            oninput: move |event: FormEvent| permission_search.set(event.value()),
                        }
                        if !permission_search().is_empty() {
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "清除权限搜索",
                                aria_label: "清除权限搜索",
                                onclick: move |_| permission_search.set(String::new()),
                                icons::X { class: "size-4" }
                            }
                        }
                    }
                    Badge { variant: BadgeVariant::Outline, "{rows.len()} / {permission_count}" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        onclick: move |_| {
                            editing_permission.set(None);
                            deleting_permission.set(None);
                            creating_permission.set(Some(PermissionDefinition {
                                id: SymbolId::new(),
                                name: String::new(),
                                title: String::new(),
                                allowed_effects: Vec::new(),
                            }));
                        },
                        icons::Plus { class: "size-4" }
                        "新建权限"
                    }
                }
            }
            DataTable::<PermissionTableRow> {
                class: "aio-permission-data-table",
                aria_label: "权限定义",
                rows,
                columns: permission_table_columns(),
                max_height: "38rem",
                empty_text: if normalized_search.is_empty() {
                    "暂无权限定义".to_owned()
                } else {
                    "没有匹配的权限".to_owned()
                },
                row_key: |row: PermissionTableRow| row.permission.id.to_string(),
                render_cell: move |cell: DataTableCellContext<PermissionTableRow>| {
                    permission_table_cell(cell, editing_permission, deleting_permission)
                }
            }
            if let Some(permission) = creating_dialog_permission {
                PermissionEditorDialog {
                    key: "create:{permission.id}",
                    permission,
                    mode: PermissionEditorMode::Create {
                        root_id,
                        index: permission_count,
                    },
                    siblings: permissions.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    creating_permission,
                    editing_permission,
                }
            } else if let Some(permission) = editing_dialog_permission {
                PermissionEditorDialog {
                    key: "edit:{permission.id}",
                    permission,
                    mode: PermissionEditorMode::Edit,
                    siblings: permissions,
                    api_base_url: api_base_url.clone(),
                    program_id: program_id.clone(),
                    version,
                    generation,
                    status,
                    creating_permission,
                    editing_permission,
                }
            }
            if let Some(row) = deleting_dialog_permission {
                PermissionDeleteDialog {
                    row,
                    api_base_url,
                    program_id,
                    version,
                    generation,
                    status,
                    deleting_permission,
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PermissionTableRow {
    permission: PermissionDefinition,
    usage_count: usize,
}

pub(super) fn permission_table_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("name", "权限标识")
            .width(240)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("title", "权限名称").width(220),
        DataTableColumn::leaf("effects", "允许 Effect").width(460),
        DataTableColumn::leaf("usage", "引用")
            .width(96)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("actions", "操作")
            .width(112)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}

pub(super) fn permission_table_cell(
    cell: DataTableCellContext<PermissionTableRow>,
    mut editing_permission: Signal<Option<SymbolId>>,
    mut deleting_permission: Signal<Option<SymbolId>>,
) -> Element {
    let permission = cell.row.permission;
    let permission_id = permission.id;
    let permission_name = permission.name.clone();
    let delete_disabled = cell.row.usage_count > 0;
    let delete_title = if delete_disabled {
        format!("该权限被 {} 处定义引用，不能删除", cell.row.usage_count)
    } else {
        format!("删除权限 {permission_name}")
    };
    match cell.column.key.as_str() {
        "name" => rsx! { code { class: "aio-permission-cell__name", "{permission.name}" } },
        "title" => rsx! { strong { "{permission.title}" } },
        "effects" => rsx! {
            div { class: "aio-permission-cell__effects",
                if permission.allowed_effects.is_empty() {
                    span { class: "aio-permission-cell__empty", "无 Effect" }
                } else {
                    for effect in permission.allowed_effects {
                        Badge { variant: BadgeVariant::Outline, "{effect.label()}" }
                    }
                }
            }
        },
        "usage" => rsx! {
            span { class: "aio-permission-cell__usage",
                if cell.row.usage_count == 0 { "未引用" } else { "{cell.row.usage_count} 处" }
            }
        },
        "actions" => rsx! {
            div { class: "aio-permission-cell__actions",
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "编辑权限 {permission_name}",
                    aria_label: "编辑权限 {permission_name}",
                    onclick: move |event: MouseEvent| {
                        event.stop_propagation();
                        deleting_permission.set(None);
                        editing_permission.set(Some(permission_id));
                    },
                    icons::Pencil { class: "size-4" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    disabled: delete_disabled,
                    title: "{delete_title}",
                    aria_label: "{delete_title}",
                    onclick: move |event: MouseEvent| {
                        event.stop_propagation();
                        editing_permission.set(None);
                        deleting_permission.set(Some(permission_id));
                    },
                    icons::Trash2 { class: "size-4" }
                }
            }
        },
        _ => rsx! { "—" },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PermissionEditorMode {
    Create { root_id: SymbolId, index: usize },
    Edit,
}

#[component]
pub(super) fn PermissionEditorDialog(
    permission: PermissionDefinition,
    mode: PermissionEditorMode,
    siblings: Vec<PermissionDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut creating_permission: Signal<Option<PermissionDefinition>>,
    mut editing_permission: Signal<Option<SymbolId>>,
) -> Element {
    let editing = matches!(mode, PermissionEditorMode::Edit);
    let permission_id = permission.id;
    let selected_effects = permission.allowed_effects.clone();
    let stable_name = permission.name;
    let mut title = use_signal(move || permission.title);
    let close_editor = use_callback(move |_: ()| match mode {
        PermissionEditorMode::Create { .. } => creating_permission.set(None),
        PermissionEditorMode::Edit => editing_permission.set(None),
    });

    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-permission-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    close_editor.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { if editing { "编辑权限" } else { "新建权限" } }
                    DialogDescription { "声明可被菜单、路由和函数复用的授权能力" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭权限编辑",
                    aria_label: "关闭权限编辑",
                    onclick: move |_| close_editor.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_title = title().trim().to_owned();
                if next_title.is_empty() {
                    status.set(Some("权限名称不能为空".to_owned()));
                    return;
                }
                let next_name = if editing {
                    stable_name.clone()
                } else {
                    let action = identifier_from_title(&next_title);
                    if action.is_empty() {
                        status.set(Some("权限名称无法生成有效标识，请包含中文、字母或数字".to_owned()));
                        return;
                    }
                    format!("permission:{action}")
                };
                if siblings
                    .iter()
                    .any(|item| item.id != permission_id && item.name == next_name)
                {
                    status.set(Some(format!("权限标识已存在: {next_name}")));
                    return;
                }
                let allowed_effects = permission_effects_from_form(&event);
                let patches = match mode {
                    PermissionEditorMode::Create { root_id, index } => {
                        let permission = PermissionDefinition {
                            id: permission_id,
                            name: next_name,
                            title: next_title,
                            allowed_effects,
                        };
                        vec![GraphPatch::Insert {
                            parent_id: root_id,
                            collection: ChildCollection::Permissions,
                            index,
                            entity: Box::new(GraphEntity::Permission(permission)),
                        }]
                    }
                    PermissionEditorMode::Edit => vec![
                        GraphPatch::Rename {
                            target_id: permission_id,
                            name: next_name,
                            title: Some(next_title),
                        },
                        GraphPatch::SetProperty {
                            target_id: permission_id,
                            property: crate::EditableProperty::PermissionEffects,
                            value: serde_json::json!(allowed_effects),
                        },
                    ],
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
                div { class: "aio-definition-dialog__grid",
                    label {
                        span { "权限名称" }
                        Input {
                            class: "aio-input",
                            aria_label: "权限名称",
                            placeholder: "例如 查看资产",
                            value: title(),
                            oninput: move |event: FormEvent| title.set(event.value()),
                        }
                    }
                }
                section { class: "aio-definition-dialog__section",
                    h3 { "允许 Effect" }
                    {permission_effect_editor(&selected_effects)}
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
                        if editing { "保存权限" } else { "创建权限" }
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn PermissionDeleteDialog(
    row: PermissionTableRow,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut deleting_permission: Signal<Option<SymbolId>>,
) -> Element {
    let permission_id = row.permission.id;
    let permission_name = row.permission.name;
    rsx! {
        Dialog {
            class: "aio-endpoint-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting_permission.set(None);
                }
            },
            DialogTitle { "删除权限" }
            DialogDescription { "确认删除权限 {permission_name}？删除后不可恢复。" }
            footer { class: "aio-endpoint-confirm-dialog__actions",
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting_permission.set(None),
                    "取消"
                }
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Destructive,
                    disabled: row.usage_count > 0,
                    onclick: move |_| {
                        if row.usage_count > 0 {
                            return;
                        }
                        submit_patches(
                            api_base_url.clone(),
                            program_id.clone(),
                            version,
                            vec![GraphPatch::Delete { target_id: permission_id }],
                            generation,
                            status,
                        );
                        deleting_permission.set(None);
                    },
                    "删除"
                }
            }
        }
    }
}

pub(super) fn permission_effect_editor(selected: &[EffectKind]) -> Element {
    rsx! {
        div { class: "aio-permission-dialog__effects",
            for effect in EffectKind::all() {
                label {
                    Checkbox {
                        name: "{permission_effect_input_name(effect)}",
                        default_checked: checkbox_state(selected.contains(&effect)),
                        aria_label: "允许 {effect.label()}",
                    }
                    span { "{effect.label()}" }
                }
            }
        }
    }
}

pub(super) fn permission_effect_input_name(effect: EffectKind) -> String {
    format!("permission_effect_{}", effect.as_str())
}

pub(super) fn permission_effects_from_form(event: &FormEvent) -> Vec<EffectKind> {
    EffectKind::all()
        .into_iter()
        .filter(|effect| !form_text(event, &permission_effect_input_name(*effect)).is_empty())
        .collect()
}
