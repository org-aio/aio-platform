use super::*;

#[component]
pub(super) fn ApplicationPanel(
    draft: DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let mut identity_open = use_signal(|| false);
    let mut preview = use_signal(|| None::<Result<ApplicationBundle, String>>);
    let mut selected_file = use_signal(|| "Cargo.toml".to_owned());
    let definition = draft.definition.clone();
    let program_id = draft.program_id.clone();
    let version = draft.version;
    let application_id = definition.name.clone();
    let application_title = definition.title.clone();
    let targets = definition.application_targets.clone();
    let preview_snapshot = preview();
    let selected_content = preview_snapshot
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(|bundle| {
            bundle
                .files
                .iter()
                .find(|file| file.path == selected_file())
                .map(|file| file.content.clone())
        })
        .unwrap_or_default();

    rsx! {
        section { class: "aio-application-management",
            header { class: "aio-application-management__toolbar",
                div {
                    h2 { "应用交付" }
                    p { "从当前 ProgramDefinition 生成独立的 Web、Desktop 与 Server 工程" }
                }
                div { class: "aio-application-management__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: {
                            let api_base_url = api_base_url.clone();
                            move |_| {
                                let api_base_url = api_base_url.clone();
                                spawn(async move {
                                    status.set(Some("正在生成源码预览".to_owned()));
                                    let result = get_api::<ApplicationBundle>(
                                        &api_base_url,
                                        "/api/studio/application/preview",
                                    )
                                    .await;
                                    if let Ok(bundle) = &result {
                                        if let Some(file) = bundle.files.first() {
                                            selected_file.set(file.path.clone());
                                        }
                                        status.set(Some(format!(
                                            "预览已生成，共 {} 个文件",
                                            bundle.files.len()
                                        )));
                                    } else if let Err(error) = &result {
                                        status.set(Some(error.clone()));
                                    }
                                    preview.set(Some(result));
                                });
                            }
                        },
                        icons::Eye { class: "size-4" }
                        "生成预览"
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        onclick: {
                            let api_base_url = api_base_url.clone();
                            move |_| {
                                let api_base_url = api_base_url.clone();
                                spawn(async move {
                                    status.set(Some("正在发布并生成应用".to_owned()));
                                    match post_api::<_, ApplicationGenerationResult>(
                                        &api_base_url,
                                        "/api/studio/application/generate",
                                        &Value::Null,
                                    )
                                    .await
                                    {
                                        Ok(result) => {
                                            status.set(Some(format!(
                                                "应用已生成到 {}，共 {} 个文件",
                                                result.path,
                                                result.files.len()
                                            )));
                                        }
                                        Err(error) => status.set(Some(error)),
                                    }
                                });
                            }
                        },
                        icons::Package { class: "size-4" }
                        "生成应用"
                    }
                }
            }

            section { class: "aio-application-management__identity border-b py-5",
                header { class: "flex items-start justify-between gap-4",
                    div {
                        h3 { "应用身份" }
                        p { "目录名同时决定 Cargo 包名与构建产物目录" }
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        title: "编辑应用身份",
                        aria_label: "编辑应用身份",
                        onclick: move |_| identity_open.set(true),
                        icons::Pencil { class: "size-4" }
                    }
                }
                dl { class: "grid gap-4 pt-4 md:grid-cols-2",
                    div {
                        dt { "应用名称" }
                        dd { strong { "{application_title}" } }
                    }
                    div {
                        dt { "目录标识" }
                        dd { code { "{application_id}" } }
                    }
                }
            }

            section { class: "aio-application-management__targets border-b py-5",
                header {
                    h3 { "客户端目标" }
                    p { "页面与 API 契约保持一份源码，平台只切换 Dioxus 渲染器和 HTTP 传输" }
                }
                div { class: "grid gap-3 pt-4 md:grid-cols-2",
                    {application_target_toggle(
                        ApplicationTarget::Web,
                        "Web",
                        "浏览器发布包与容器静态资源",
                        &targets,
                        definition.id,
                        api_base_url.clone(),
                        program_id.clone(),
                        version,
                        generation,
                        status,
                    )}
                    {application_target_toggle(
                        ApplicationTarget::Desktop,
                        "Desktop",
                        "macOS、Windows 与 Linux 原生桌面壳",
                        &targets,
                        definition.id,
                        api_base_url.clone(),
                        program_id.clone(),
                        version,
                        generation,
                        status,
                    )}
                }
            }

            section { class: "aio-application-management__preview py-5",
                header {
                    h3 { "生成预览" }
                    p { "预览与实际落盘使用同一个确定性 ApplicationCompiler" }
                }
                match preview_snapshot {
                    Some(Ok(bundle)) => rsx! {
                        div { class: "grid gap-4 pt-4 lg:grid-cols-[18rem_minmax(0,1fr)]",
                            label {
                                span { "文件" }
                                select {
                                    class: "aio-input",
                                    value: selected_file(),
                                    onchange: move |event: FormEvent| selected_file.set(event.value()),
                                    for file in bundle.files {
                                        option { value: "{file.path}", "{file.path}" }
                                    }
                                }
                            }
                            pre { class: "max-h-[34rem] overflow-auto border p-4 text-xs",
                                code { "{selected_content}" }
                            }
                        }
                    },
                    Some(Err(error)) => empty_panel(&error),
                    None => empty_panel("点击生成预览检查待交付文件"),
                }
            }

            if identity_open() {
                ApplicationIdentityDialog {
                    definition,
                    api_base_url,
                    program_id,
                    version,
                    generation,
                    status,
                    on_close: move |_| identity_open.set(false),
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn application_target_toggle(
    target: ApplicationTarget,
    title: &'static str,
    description: &'static str,
    targets: &BTreeSet<ApplicationTarget>,
    definition_id: SymbolId,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let checked = targets.contains(&target);
    let current_targets = targets.clone();
    rsx! {
        label { class: "flex items-center justify-between gap-4 border p-4",
            div {
                strong { "{title}" }
                p { "{description}" }
            }
            Checkbox {
                checked: Some(checkbox_state(checked)),
                aria_label: "切换 {title} 发布目标",
                on_checked_change: move |state| {
                    let mut next = current_targets.clone();
                    if checkbox_is_checked(state) {
                        next.insert(target);
                    } else {
                        next.remove(&target);
                    }
                    if next.is_empty() {
                        status.set(Some("应用至少需要一个客户端发布目标".to_owned()));
                        return;
                    }
                    let value = match serde_json::to_value(next) {
                        Ok(value) => value,
                        Err(error) => {
                            status.set(Some(format!("序列化应用目标失败: {error}")));
                            return;
                        }
                    };
                    submit_patches(
                        api_base_url.clone(),
                        program_id.clone(),
                        version,
                        vec![GraphPatch::SetProperty {
                            target_id: definition_id,
                            property: crate::EditableProperty::ApplicationTargets,
                            value,
                        }],
                        generation,
                        status,
                    );
                },
            }
        }
    }
}

#[component]
fn ApplicationIdentityDialog(
    definition: crate::ProgramDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
) -> Element {
    let initial_name = definition.name.clone();
    let initial_title = definition.title.clone();
    let definition_id = definition.id;
    let mut name = use_signal(move || initial_name);
    let mut title = use_signal(move || initial_title);
    rsx! {
        Dialog {
            class: "aio-definition-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "编辑应用身份" }
                    DialogDescription { "目录标识会成为 apps 下的生成目录和 Cargo 包名后缀" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭应用身份编辑",
                    aria_label: "关闭应用身份编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let next_name = name().trim().to_owned();
                let next_title = title().trim().to_owned();
                if next_title.is_empty() {
                    status.set(Some("应用名称不能为空".to_owned()));
                    return;
                }
                let valid_name = next_name
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_lowercase())
                    && next_name.chars().all(|character| {
                        character.is_ascii_lowercase()
                            || character.is_ascii_digit()
                            || character == '-'
                    });
                if !valid_name {
                    status.set(Some("目录标识必须以小写字母开头，且只能包含小写字母、数字和连字符".to_owned()));
                    return;
                }
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    vec![GraphPatch::Rename {
                        target_id: definition_id,
                        name: next_name,
                        title: Some(next_title),
                    }],
                    generation,
                    status,
                );
                on_close.call(());
            },
                label {
                    span { "应用名称" }
                    Input {
                        class: "aio-input",
                        name: "application_title",
                        value: title(),
                        oninput: move |event: FormEvent| title.set(event.value()),
                    }
                }
                label {
                    span { "目录标识" }
                    Input {
                        class: "aio-input",
                        name: "application_name",
                        value: name(),
                        oninput: move |event: FormEvent| name.set(event.value()),
                    }
                }
                footer { class: "aio-definition-dialog__footer",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Outline,
                        onclick: move |_| on_close.call(()),
                        "取消"
                    }
                    Button { r#type: "submit", "保存" }
                }
            }
        }
    }
}
