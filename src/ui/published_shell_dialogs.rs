use super::*;

/// 发布应用壳只编辑应用显示标题，不改变生成目录标识。
#[component]
pub(crate) fn AdminApplicationTitleEditor(
    api_base_url: String,
    current_title: String,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor_open: Signal<bool>,
) -> Element {
    let mut title = use_signal(move || current_title);
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    let Some(result) = draft.read().as_ref().cloned() else {
        return title_editor_state("正在加载应用定义", editor_open);
    };
    let draft = match result {
        Ok(draft) => draft,
        Err(error) => return title_editor_state(&error, editor_open),
    };
    let program_id = draft.program_id.clone();
    let definition_id = draft.definition.id;
    rsx! {
        Dialog {
            class: "aio-definition-dialog",
            open: true,
            on_open_change: move |open| editor_open.set(open),
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "编辑应用标题" }
                    DialogDescription { "标题显示在工作台左上角，目录标识保持不变" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭应用标题编辑",
                    aria_label: "关闭应用标题编辑",
                    onclick: move |_| editor_open.set(false),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_title = title().trim().to_owned();
                if next_title.is_empty() {
                    status.set(Some("应用标题不能为空".to_owned()));
                    return;
                }
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    draft.version,
                    vec![GraphPatch::SetProperty {
                        target_id: definition_id,
                        property: crate::EditableProperty::Title,
                        value: serde_json::json!(next_title),
                    }],
                    generation,
                    status,
                );
                editor_open.set(false);
            },
                label { r#for: "admin-application-title", "应用标题" }
                Input {
                    id: "admin-application-title",
                    name: "title",
                    class: "aio-input",
                    aria_label: "应用标题",
                    value: title(),
                    oninput: move |event: FormEvent| title.set(event.value()),
                }
                if let Some(message) = status() {
                    p { class: "text-xs text-destructive", role: "alert", "{message}" }
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| editor_open.set(false),
                        "取消"
                    }
                    Button { r#type: "submit", "保存" }
                }
            }
        }
    }
}

/// 发布应用壳删除菜单项时复用 Studio 的依赖分析与确认流程。
#[component]
pub(crate) fn AdminMenuDeleteDialog(
    api_base_url: String,
    menu_id: SymbolId,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    deleting_menu: Signal<Option<SymbolId>>,
    on_deleted: EventHandler<()>,
) -> Element {
    let draft_api = api_base_url.clone();
    let draft = use_resource(move || {
        let api_base_url = draft_api.clone();
        let _generation = generation();
        async move { get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await }
    });
    let Some(result) = draft.read().as_ref().cloned() else {
        return menu_delete_state("正在加载菜单定义", deleting_menu);
    };
    let draft = match result {
        Ok(draft) => draft,
        Err(error) => return menu_delete_state(&error, deleting_menu),
    };
    let Some(row) = find_menu_table_row(&draft.definition.menus, draft.definition.id, menu_id)
    else {
        return menu_delete_state("菜单已经不存在", deleting_menu);
    };
    rsx! {
        MenuDeleteDialog {
            row,
            menus: draft.definition.menus,
            routes: draft.definition.routes,
            api_base_url,
            program_id: draft.program_id,
            version: draft.version,
            generation,
            status,
            deleting_menu,
            on_deleted,
        }
    }
}

fn title_editor_state(message: &str, mut editor_open: Signal<bool>) -> Element {
    rsx! {
        Dialog {
            class: "aio-definition-dialog",
            open: true,
            on_open_change: move |open| editor_open.set(open),
            DialogTitle { "编辑应用标题" }
            p { role: "status", "{message}" }
            footer {
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| editor_open.set(false),
                    "关闭"
                }
            }
        }
    }
}

fn menu_delete_state(message: &str, mut deleting_menu: Signal<Option<SymbolId>>) -> Element {
    rsx! {
        Dialog {
            class: "aio-endpoint-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting_menu.set(None);
                }
            },
            DialogTitle { "删除菜单项" }
            p { role: "status", "{message}" }
            footer {
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting_menu.set(None),
                    "关闭"
                }
            }
        }
    }
}
