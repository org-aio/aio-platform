use super::*;

#[component]
pub(super) fn EndpointEditor(
    mut draft: Signal<PageEndpointDefinition>,
    siblings: Vec<PageEndpointDefinition>,
    stable_input_names: BTreeMap<SymbolId, String>,
    stable_output_names: BTreeMap<SymbolId, String>,
    mut status: Signal<Option<String>>,
    on_submit: EventHandler<PageEndpointDefinition>,
    on_cancel: EventHandler<()>,
) -> Element {
    let initial_path = draft().path.clone();
    let mut path = use_signal(move || initial_path);
    let mut endpoint = draft();
    endpoint.path = path();
    normalize_endpoint_parameter_names(&mut endpoint, &stable_input_names, &stable_output_names);
    let errors = validate_page_endpoint_draft(&endpoint, &siblings);
    let can_save = errors.is_empty();
    let submit_stable_input_names = stable_input_names.clone();
    let submit_stable_output_names = stable_output_names.clone();
    let input_stable_names = stable_input_names;
    let output_stable_names = stable_output_names;
    rsx! {
        form { class: "aio-endpoint-editor", onsubmit: move |event| {
            event.prevent_default();
            let mut endpoint = draft();
            endpoint.path = path();
            normalize_endpoint_parameter_names(
                &mut endpoint,
                &submit_stable_input_names,
                &submit_stable_output_names,
            );
            let errors = validate_page_endpoint_draft(&endpoint, &siblings);
            if let Some(error) = errors.first() {
                status.set(Some(error.clone()));
                return;
            }
            on_submit.call(endpoint);
        },
            header { class: "aio-endpoint-editor__header",
                div { class: "aio-endpoint-request-line",
                    select {
                        class: "aio-input aio-endpoint-method",
                        aria_label: "HTTP 方法",
                        onchange: move |event: FormEvent| {
                            draft.with_mut(|endpoint| {
                                endpoint.method = rest_method_from_key(&event.value());
                            });
                        },
                        {rest_method_options(endpoint.method)}
                    }
                    Input {
                        class: "aio-input aio-endpoint-path",
                        value: path(),
                        aria_label: "REST 路径",
                        placeholder: "/api/users/batch-disable",
                        oninput: move |event: FormEvent| {
                            path.set(event.value());
                            draft.with_mut(|endpoint| {
                                endpoint.path = event.value();
                                synchronize_path_parameter_names(endpoint);
                            });
                        }
                    }
                }
            }
            div { class: "aio-endpoint-editor__identity",
                label { "显示名称（可选）"
                    Input {
                        class: "aio-input",
                        value: "{endpoint.title}",
                        oninput: move |event: FormEvent| {
                            draft.with_mut(|endpoint| endpoint.title = event.value());
                        }
                    }
                }
                label { "详细说明"
                    Textarea {
                        class: "aio-input",
                        rows: "3",
                        value: "{endpoint.description}",
                        oninput: move |event: FormEvent| {
                            draft.with_mut(|endpoint| endpoint.description = event.value());
                        }
                    }
                }
            }
            section { class: "aio-endpoint-parameters",
                header {
                    strong { "入参" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| {
                            draft.with_mut(|endpoint| {
                                let title = format!("入参 {}", endpoint.inputs.len() + 1);
                                let name = unique_identifier_from_title(
                                    &title,
                                    "input",
                                    endpoint.inputs.iter().map(|input| input.name.as_str()),
                                );
                                endpoint.inputs.push(EndpointInputDefinition {
                                    id: SymbolId::new(),
                                    name,
                                    title,
                                    location: EndpointInputLocation::Body,
                                    value_type: ValueType::Text,
                                    required: false,
                                });
                            });
                        },
                        icons::Plus { class: "size-4" }
                        "入参"
                    }
                }
                DataTable::<EndpointInputDefinition> {
                    class: "aio-endpoint-parameter-data-table",
                    aria_label: "接口入参草稿",
                    rows: endpoint.inputs.clone(),
                    columns: endpoint_input_columns(),
                    max_height: "18rem",
                    row_key: |input: EndpointInputDefinition| input.id.to_string(),
                    render_cell: move |cell: DataTableCellContext<EndpointInputDefinition>| {
                        endpoint_input_draft_cell(cell, draft, path, input_stable_names.clone())
                    },
                    empty_text: "无入参",
                }
            }
            section { class: "aio-endpoint-parameters",
                header {
                    strong { "响应 data" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| {
                            draft.with_mut(|endpoint| {
                                let title = format!("响应字段 {}", endpoint.outputs.len() + 1);
                                let name = unique_identifier_from_title(
                                    &title,
                                    "output",
                                    endpoint.outputs.iter().map(|output| output.name.as_str()),
                                );
                                endpoint.outputs.push(EndpointOutputDefinition {
                                    id: SymbolId::new(),
                                    name,
                                    title,
                                    value_type: ValueType::Text,
                                });
                            });
                        },
                        icons::Plus { class: "size-4" }
                        "出参"
                    }
                }
                DataTable::<EndpointOutputDefinition> {
                    class: "aio-endpoint-parameter-data-table",
                    aria_label: "接口响应草稿",
                    rows: endpoint.outputs.clone(),
                    columns: endpoint_output_columns(),
                    max_height: "18rem",
                    row_key: |output: EndpointOutputDefinition| output.id.to_string(),
                    render_cell: move |cell: DataTableCellContext<EndpointOutputDefinition>| {
                        endpoint_output_draft_cell(cell, draft, output_stable_names.clone())
                    },
                    empty_text: "无响应字段",
                }
            }
            if !errors.is_empty() {
                div { class: "aio-endpoint-editor__diagnostics", role: "alert",
                    strong { "保存前检查" }
                    ul {
                        for error in &errors {
                            li { "{error}" }
                        }
                    }
                }
            }
            footer {
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| on_cancel.call(()),
                    "取消"
                }
                Button { r#type: "submit", disabled: !can_save,
                    icons::Save { class: "size-4" }
                    "保存接口"
                }
            }
        }
    }
}

pub(super) fn endpoint_input_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("name", "自动标识").width(150),
        DataTableColumn::leaf("title", "显示标题").width(180),
        DataTableColumn::leaf("location", "位置").width(110),
        DataTableColumn::leaf("type", "类型").width(130),
        DataTableColumn::leaf("required", "必填")
            .width(72)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("actions", "操作")
            .width(64)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}

pub(super) fn endpoint_output_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("name", "自动标识").width(170),
        DataTableColumn::leaf("title", "显示标题").width(220),
        DataTableColumn::leaf("type", "类型").width(150),
        DataTableColumn::leaf("actions", "操作")
            .width(64)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}

pub(super) fn endpoint_input_draft_cell(
    cell: DataTableCellContext<EndpointInputDefinition>,
    mut draft: Signal<PageEndpointDefinition>,
    path: Signal<String>,
    stable_names: BTreeMap<SymbolId, String>,
) -> Element {
    let input = cell.row;
    let input_id = input.id;
    match cell.column.key.as_str() {
        "name" => rsx! { code { "{input.name}" } },
        "title" => rsx! {
            Input {
                class: "aio-input aio-endpoint-parameter-input",
                value: "{input.title}",
                aria_label: "入参显示标题",
                oninput: move |event: FormEvent| {
                    let title = event.value();
                    let generated_name = (!stable_names.contains_key(&input_id)
                        && input.location != EndpointInputLocation::Path)
                        .then(|| {
                        unique_identifier_from_title(
                            &title,
                            "input",
                            draft()
                                .inputs
                                .iter()
                                .filter(|candidate| candidate.id != input_id)
                                .map(|candidate| candidate.name.as_str()),
                        )
                    });
                    update_endpoint_input(&mut draft, input_id, |input| {
                        input.title = title;
                        if let Some(name) = generated_name {
                            input.name = name;
                        }
                    });
                },
            }
        },
        "location" => rsx! {
            select {
                class: "aio-input aio-endpoint-parameter-input",
                aria_label: "入参位置",
                onchange: move |event: FormEvent| {
                    let location = endpoint_location_from_key(&event.value());
                    let path_name = (location == EndpointInputLocation::Path).then(|| {
                        next_endpoint_path_parameter_name(
                            &path(),
                            draft()
                                .inputs
                                .iter()
                                .filter(|candidate| candidate.id != input_id)
                                .map(|candidate| candidate.name.as_str()),
                        )
                    }).flatten();
                    update_endpoint_input(&mut draft, input_id, |input| {
                        input.location = location;
                        input.required = location == EndpointInputLocation::Path || input.required;
                        if let Some(name) = path_name {
                            input.name = name;
                        }
                    });
                },
                {endpoint_location_options(input.location)}
            }
        },
        "type" => {
            let current_type = input.value_type.clone();
            let selected = editable_value_type_key(&current_type).to_owned();
            rsx! {
                select {
                    class: "aio-input aio-endpoint-parameter-input",
                    aria_label: "入参类型",
                    onchange: move |event: FormEvent| update_endpoint_input(&mut draft, input_id, |input| {
                        input.value_type = editable_value_type_from_key(&event.value(), &current_type);
                    }),
                    {editable_value_type_options(&input.value_type, selected)}
                }
            }
        }
        "required" => rsx! {
            div { class: "aio-endpoint-parameter-required",
                Checkbox {
                    checked: Some(checkbox_state(input.required)),
                    aria_label: "入参必填",
                    on_checked_change: move |checked| update_endpoint_input(&mut draft, input_id, |input| {
                        input.required = checkbox_is_checked(checked);
                    }),
                }
            }
        },
        "actions" => rsx! {
            div { class: "aio-endpoint-table__actions",
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "移除入参草稿",
                    aria_label: "移除入参草稿",
                    onclick: move |_| draft.with_mut(|endpoint| {
                        endpoint.inputs.retain(|input| input.id != input_id);
                    }),
                    icons::Trash2 { class: "size-4" }
                }
            }
        },
        _ => rsx! { "—" },
    }
}

pub(super) fn endpoint_output_draft_cell(
    cell: DataTableCellContext<EndpointOutputDefinition>,
    mut draft: Signal<PageEndpointDefinition>,
    stable_names: BTreeMap<SymbolId, String>,
) -> Element {
    let output = cell.row;
    let output_id = output.id;
    match cell.column.key.as_str() {
        "name" => rsx! { code { "{output.name}" } },
        "title" => rsx! {
            Input {
                class: "aio-input aio-endpoint-parameter-input",
                value: "{output.title}",
                aria_label: "响应字段显示标题",
                oninput: move |event: FormEvent| {
                    let title = event.value();
                    let generated_name = (!stable_names.contains_key(&output_id)).then(|| {
                        unique_identifier_from_title(
                            &title,
                            "output",
                            draft()
                                .outputs
                                .iter()
                                .filter(|candidate| candidate.id != output_id)
                                .map(|candidate| candidate.name.as_str()),
                        )
                    });
                    update_endpoint_output(&mut draft, output_id, |output| {
                        output.title = title;
                        if let Some(name) = generated_name {
                            output.name = name;
                        }
                    });
                },
            }
        },
        "type" => {
            let current_type = output.value_type.clone();
            let selected = editable_value_type_key(&current_type).to_owned();
            rsx! {
                select {
                    class: "aio-input aio-endpoint-parameter-input",
                    aria_label: "响应字段类型",
                    onchange: move |event: FormEvent| update_endpoint_output(&mut draft, output_id, |output| {
                        output.value_type = editable_value_type_from_key(&event.value(), &current_type);
                    }),
                    {editable_value_type_options(&output.value_type, selected)}
                }
            }
        }
        "actions" => rsx! {
            div { class: "aio-endpoint-table__actions",
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "移除响应字段草稿",
                    aria_label: "移除响应字段草稿",
                    onclick: move |_| draft.with_mut(|endpoint| {
                        endpoint.outputs.retain(|output| output.id != output_id);
                    }),
                    icons::Trash2 { class: "size-4" }
                }
            }
        },
        _ => rsx! { "—" },
    }
}

pub(super) fn update_endpoint_input(
    draft: &mut Signal<PageEndpointDefinition>,
    input_id: SymbolId,
    update: impl FnOnce(&mut EndpointInputDefinition),
) {
    draft.with_mut(|endpoint| {
        if let Some(input) = endpoint
            .inputs
            .iter_mut()
            .find(|input| input.id == input_id)
        {
            update(input);
        }
    });
}

pub(super) fn update_endpoint_output(
    draft: &mut Signal<PageEndpointDefinition>,
    output_id: SymbolId,
    update: impl FnOnce(&mut EndpointOutputDefinition),
) {
    draft.with_mut(|endpoint| {
        if let Some(output) = endpoint
            .outputs
            .iter_mut()
            .find(|output| output.id == output_id)
        {
            update(output);
        }
    });
}

pub(super) fn submit_endpoint_update(
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) {
    submit_endpoint_definition(
        EndpointEditorMode::Edit,
        endpoint,
        api_base_url,
        program_id,
        version,
        generation,
        status,
    );
}

pub(super) fn submit_endpoint_definition(
    mode: EndpointEditorMode,
    endpoint: PageEndpointDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) {
    let patch = match mode {
        EndpointEditorMode::Create { page_id, index } => GraphPatch::Insert {
            parent_id: page_id,
            collection: ChildCollection::PageEndpoints,
            index,
            entity: GraphEntity::PageEndpoint(endpoint),
        },
        EndpointEditorMode::Edit => {
            let endpoint_id = endpoint.id;
            let value = match serde_json::to_value(endpoint) {
                Ok(value) => value,
                Err(error) => {
                    status.set(Some(format!("序列化接口失败: {error}")));
                    return;
                }
            };
            GraphPatch::SetProperty {
                target_id: endpoint_id,
                property: crate::EditableProperty::PageEndpoint,
                value,
            }
        }
    };
    submit_patches(
        api_base_url,
        program_id,
        version,
        vec![patch],
        generation,
        status,
    );
}

pub(super) fn generate_endpoint_with_ai(
    api_base_url: String,
    page_id: SymbolId,
    page_title: String,
    version: i64,
    intent: String,
    mut generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) {
    spawn(async move {
        let prompt = format!(
            "只为页面 {page_title}（SymbolId: {page_id}）新增一个自定义 REST 接口。\
             必须使用 GraphPatch::Insert，parent_id 为该页面，collection 为 page_endpoints，\
             entity 为 page_endpoint。根据中文需求生成可选中文显示名、HTTP 方法、本应用相对路径、\
             详细 description、完整 inputs 和 outputs，并固定生成 implementation={{\"kind\":\"convention\"}}；\
             REST 路径就是接口标识，不得新增 name 或 intent 字段。\
             路径参数必须在 path 中使用同名花括号。中文需求只用于本次生成：{intent}"
        );
        let request = VibeRunRequest {
            prompt,
            model: None,
        };
        match post_api::<_, VibeRunAccepted>(
            &api_base_url,
            "/api/studio/program/vibe-runs",
            &request,
        )
        .await
        {
            Ok(_) => status.set(Some("正在生成接口元数据".to_owned())),
            Err(error) => {
                status.set(Some(error));
                return;
            }
        }
        for _ in 0..60 {
            TimeoutFuture::new(1_000).await;
            match get_api::<DraftSnapshot>(&api_base_url, "/api/studio/program/draft").await {
                Ok(draft) if draft.version > version => {
                    generation.with_mut(|value| *value = value.saturating_add(1));
                    status.set(Some("接口元数据已生成".to_owned()));
                    return;
                }
                Ok(_) | Err(_) => {}
            }
        }
        status.set(Some("接口仍在生成，可稍后重新打开页面设置查看".to_owned()));
    });
}

pub(super) fn rest_method_options(selected: RestMethod) -> Element {
    rsx! {
        for method in [RestMethod::Get, RestMethod::Post, RestMethod::Put, RestMethod::Patch, RestMethod::Delete] {
            option { value: method.as_str(), selected: method == selected, "{method.as_str()}" }
        }
    }
}

pub(super) fn rest_method_from_key(value: &str) -> RestMethod {
    match value {
        "GET" => RestMethod::Get,
        "PUT" => RestMethod::Put,
        "PATCH" => RestMethod::Patch,
        "DELETE" => RestMethod::Delete,
        _ => RestMethod::Post,
    }
}

pub(super) fn endpoint_location_options(selected: EndpointInputLocation) -> Element {
    rsx! {
        option { value: "path", selected: selected == EndpointInputLocation::Path, "Path" }
        option { value: "query", selected: selected == EndpointInputLocation::Query, "Query" }
        option { value: "header", selected: selected == EndpointInputLocation::Header, "Header" }
        option { value: "body", selected: selected == EndpointInputLocation::Body, "Body" }
    }
}

pub(super) fn endpoint_location_from_key(value: &str) -> EndpointInputLocation {
    match value {
        "path" => EndpointInputLocation::Path,
        "query" => EndpointInputLocation::Query,
        "header" => EndpointInputLocation::Header,
        _ => EndpointInputLocation::Body,
    }
}

pub(super) fn method_class(method: RestMethod) -> &'static str {
    match method {
        RestMethod::Get => "aio-http-method aio-http-method--get",
        RestMethod::Post => "aio-http-method aio-http-method--post",
        RestMethod::Put | RestMethod::Patch => "aio-http-method aio-http-method--write",
        RestMethod::Delete => "aio-http-method aio-http-method--delete",
    }
}

pub(super) fn short_provider_key(provider_key: &str) -> &str {
    provider_key
        .rsplit_once("::")
        .map_or(provider_key, |(_, name)| name)
}
