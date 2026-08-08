use super::*;

#[component]
pub(super) fn EndpointEditorDialog(
    endpoint: PageEndpointDefinition,
    mode: EndpointEditorMode,
    siblings: Vec<PageEndpointDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut creating_endpoint: Signal<Option<PageEndpointDefinition>>,
    mut editing_endpoint: Signal<Option<SymbolId>>,
) -> Element {
    let editor_draft = use_signal(move || endpoint);
    let current = editor_draft();
    let dialog_title = if matches!(mode, EndpointEditorMode::Create { .. }) {
        "新增接口"
    } else {
        "编辑接口"
    };
    let close_editor = use_callback(move |_: ()| match mode {
        EndpointEditorMode::Create { .. } => creating_endpoint.set(None),
        EndpointEditorMode::Edit => editing_endpoint.set(None),
    });
    let submit_api = api_base_url;
    let submit_program = program_id;
    let submit_siblings = siblings.clone();
    let submit_editor = use_callback(move |endpoint: PageEndpointDefinition| {
        let errors = validate_page_endpoint_draft(&endpoint, &submit_siblings);
        if let Some(error) = errors.first() {
            let mut status = status;
            status.set(Some(error.clone()));
            return;
        }
        submit_endpoint_definition(
            mode,
            endpoint,
            submit_api.clone(),
            submit_program.clone(),
            version,
            generation,
            status,
        );
        close_editor.call(());
    });
    rsx! {
        Dialog {
            class: "aio-endpoint-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    close_editor.call(());
                }
            },
            header { class: "aio-endpoint-dialog__header",
                div { class: "aio-endpoint-dialog__heading",
                    DialogTitle { "{dialog_title}" }
                    DialogDescription { "{current.method.as_str()} {current.path}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭接口编辑",
                    aria_label: "关闭接口编辑",
                    onclick: move |_| close_editor.call(()),
                    icons::X { class: "size-4" }
                }
            }
            div { class: "aio-endpoint-dialog__body",
                EndpointEditor {
                    draft: editor_draft,
                    siblings,
                    status,
                    on_submit: move |endpoint| submit_editor.call(endpoint),
                    on_cancel: move |_| close_editor.call(()),
                }
            }
        }
    }
}

#[component]
pub(super) fn EndpointDeleteDialog(
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut editing_endpoint: Signal<Option<SymbolId>>,
    mut deleting_endpoint: Signal<Option<SymbolId>>,
) -> Element {
    let endpoint_id = endpoint.id;
    let method = endpoint.method.as_str();
    let path = endpoint.path;
    rsx! {
        Dialog {
            class: "aio-endpoint-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting_endpoint.set(None);
                }
            },
            DialogTitle { "删除接口" }
            DialogDescription {
                "确认删除 {method} {path}？对应的后端约定契约文件也会同步删除，操作不可恢复。"
            }
            footer { class: "aio-endpoint-confirm-dialog__actions",
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting_endpoint.set(None),
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
                            vec![GraphPatch::Delete { target_id: endpoint_id }],
                            generation,
                            status,
                        );
                        if editing_endpoint() == Some(endpoint_id) {
                            editing_endpoint.set(None);
                        }
                        deleting_endpoint.set(None);
                    },
                    "删除"
                }
            }
        }
    }
}

#[component]
pub(super) fn EndpointInlineCellEditor(
    edit: DataTableEditContext<EndpointTableRow>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let Some(endpoint) = edit.cell.row.definition.clone() else {
        return rsx! { "—" };
    };
    let field = edit.cell.column.key.clone();
    let initial_value = match field.as_str() {
        "path" => endpoint.path.clone(),
        "title" => endpoint.title.clone(),
        _ => return rsx! { "—" },
    };
    let mut value = use_signal(move || initial_value.clone());
    let mut submitted = use_signal(|| false);
    let close = edit.close;
    let submit_field = field.clone();
    let submit = use_callback(move |_: ()| {
        if submitted() {
            return;
        }
        let next_value = value().trim().to_owned();
        if submit_field == "path" && !next_value.starts_with('/') {
            status.set(Some("REST 路径必须以 / 开头".to_owned()));
            return;
        }
        let current_value = if submit_field == "path" {
            endpoint.path.as_str()
        } else {
            endpoint.title.as_str()
        };
        if next_value == current_value {
            close.call(());
            return;
        }
        submitted.set(true);
        let mut updated = endpoint.clone();
        if submit_field == "path" {
            updated.path = next_value;
        } else {
            updated.title = next_value;
        }
        submit_endpoint_update(
            updated,
            api_base_url.clone(),
            program_id.clone(),
            version,
            generation,
            status,
        );
        close.call(());
    });

    rsx! {
        Input {
            class: "aio-input aio-endpoint-inline-editor",
            value: value(),
            aria_label: if field == "path" { "编辑 REST 路径" } else { "编辑显示名称" },
            onmounted: move |event: MountedEvent| async move {
                let _ = event.data().set_focus(true).await;
            },
            oninput: move |event: FormEvent| value.set(event.value()),
            onblur: move |_: FocusEvent| submit.call(()),
            onkeydown: move |event: KeyboardEvent| match event.key() {
                Key::Enter => {
                    event.prevent_default();
                    submit.call(());
                }
                Key::Escape => {
                    event.prevent_default();
                    close.call(());
                }
                _ => {}
            },
        }
    }
}
