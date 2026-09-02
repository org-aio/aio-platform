fn endpoint_action_button(
    endpoint: CompiledPageEndpoint,
    mut endpoint_dialog: Signal<Option<CompiledPageEndpoint>>,
) -> Element {
    let title = endpoint.title.clone();
    rsx! {
        Button {
            variant: ButtonVariant::Outline,
            onclick: move |_| endpoint_dialog.set(Some(endpoint.clone())),
            Play { class: "size-4" }
            "{title}"
        }
    }
}

#[component]
fn RuntimeEndpointDialog(
    api_base_url: String,
    endpoint: CompiledPageEndpoint,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        Dialog {
            class: "aio-runtime-dialog aio-runtime-dialog--endpoint",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header {
                div {
                    DialogTitle { "{endpoint.title}" }
                    DialogDescription {
                        code { "{endpoint.method.as_str()} {endpoint.path}" }
                    }
                }
                Button {
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭接口调用",
                    aria_label: "关闭接口调用",
                    onclick: move |_| on_close.call(()),
                    X { class: "size-4" }
                }
            }
            RestEndpointForm {
                api_base_url,
                endpoint: endpoint.clone(),
            }
        }
    }
}

#[component]
fn RestEndpointForm(api_base_url: String, endpoint: CompiledPageEndpoint) -> Element {
    let mut response = use_signal(|| None::<Result<String, String>>);
    let mut sending = use_signal(|| false);
    let request_api = api_base_url;
    let request_endpoint = endpoint.clone();
    rsx! {
        form { class: "aio-rest-endpoint-form", onsubmit: move |event| {
            event.prevent_default();
            let values = request_endpoint
                .inputs
                .iter()
                .map(|input| (input.name.clone(), form_text(&event, &input.name)))
                .collect::<BTreeMap<_, _>>();
            let api_base_url = request_api.clone();
            let endpoint = request_endpoint.clone();
            sending.set(true);
            response.set(None);
            spawn(async move {
                let result = send_rest_endpoint_request(&api_base_url, &endpoint, &values).await;
                response.set(Some(result));
                sending.set(false);
            });
        },
            div { class: "aio-rest-endpoint-form__inputs",
                for input in &endpoint.inputs {
                    label {
                        span {
                            "{input.title}"
                            code { "{endpoint_location_name(input.location)}" }
                        }
                        RestEndpointInput {
                            key: "endpoint-input:{input.name}",
                            input: input.clone(),
                        }
                    }
                }
                if endpoint.inputs.is_empty() {
                    div { class: "aio-runtime-table-state", "无入参" }
                }
            }
            if !endpoint.outputs.is_empty() {
                dl { class: "aio-rest-endpoint-form__outputs",
                    for output in &endpoint.outputs {
                        div {
                            dt { "{output.title}" }
                            dd { code { "{output.name}" } span { "{value_type_name(&output.value_type)}" } }
                        }
                    }
                }
            }
            footer {
                Button { r#type: "submit", disabled: sending(),
                    if sending() { "发送中" } else { "发送请求" }
                }
            }
            if let Some(result) = response() {
                match result {
                    Ok(payload) => rsx! { pre { class: "aio-rest-endpoint-form__response", "{payload}" } },
                    Err(error) => rsx! { div { class: "aio-runtime-table-state is-error", "{error}" } },
                }
            }
        }
    }
}

#[component]
fn RestEndpointInput(input: crate::CompiledEndpointInput) -> Element {
    let mut selected_boolean = use_signal(String::new);
    let input_type = match input.value_type {
        ValueType::Integer | ValueType::Decimal | ValueType::TimestampMs => "number",
        ValueType::File => "file",
        _ => "text",
    };
    if input.value_type == ValueType::Boolean {
        return rsx! {
            Select {
                name: input.name,
                class: "aio-input",
                aria_required: input.required,
                aria_label: input.title,
                value: selected_boolean(),
                options: vec![
                    SelectItem::new("", "选择"),
                    SelectItem::new("true", "是"),
                    SelectItem::new("false", "否"),
                ],
                on_value_change: move |value: String| selected_boolean.set(value),
            }
        };
    }
    rsx! {
        Input {
            name: "{input.name}",
            class: "aio-input",
            r#type: input_type,
            required: input.required,
            placeholder: "{input.name}"
        }
    }
}

async fn send_rest_endpoint_request(
    api_base_url: &str,
    endpoint: &CompiledPageEndpoint,
    values: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut path = endpoint.path.clone();
    let mut query = Vec::new();
    let mut headers = Vec::new();
    let mut body = Map::new();
    for input in &endpoint.inputs {
        let value = values.get(&input.name).cloned().unwrap_or_default();
        if value.is_empty() && !input.required {
            continue;
        }
        match input.location {
            EndpointInputLocation::Path => {
                path = path.replace(&format!("{{{}}}", input.name), &urlencoding::encode(&value));
            }
            EndpointInputLocation::Query => query.push(format!(
                "{}={}",
                urlencoding::encode(&input.name),
                urlencoding::encode(&value)
            )),
            EndpointInputLocation::Header => headers.push((input.name.clone(), value)),
            EndpointInputLocation::Body => {
                body.insert(
                    input.name.clone(),
                    rest_input_value(&input.value_type, &value)?,
                );
            }
        }
    }
    if !query.is_empty() {
        let separator = if path.contains('?') { '&' } else { '?' };
        path.push(separator);
        path.push_str(&query.join("&"));
    }
    let url = api_url(api_base_url, &path);
    let body = (!body.is_empty()).then_some(Value::Object(body));
    let (status, text) =
        crate::browser_http::send_http(endpoint.method.as_str(), &url, &headers, body.as_ref())
            .await?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {text}"));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => {
            serde_json::to_string_pretty(&value).map_err(|error| format!("格式化响应失败: {error}"))
        }
        Err(_) => Ok(text),
    }
}

fn rest_input_value(value_type: &ValueType, value: &str) -> Result<Value, String> {
    match value_type {
        ValueType::Boolean => value
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|error| format!("布尔值无效: {error}")),
        ValueType::Integer | ValueType::TimestampMs => value
            .parse::<i64>()
            .map(Value::from)
            .map_err(|error| format!("整数值无效: {error}")),
        ValueType::Decimal => value
            .parse::<f64>()
            .map(Value::from)
            .map_err(|error| format!("小数值无效: {error}")),
        ValueType::Any
        | ValueType::Object { .. }
        | ValueType::List { .. }
        | ValueType::Optional { .. } => {
            serde_json::from_str(value).map_err(|error| format!("JSON 值无效: {error}"))
        }
        ValueType::Null => Ok(Value::Null),
        ValueType::Text | ValueType::File => Ok(Value::String(value.to_owned())),
    }
}

const fn endpoint_location_name(location: EndpointInputLocation) -> &'static str {
    match location {
        EndpointInputLocation::Path => "Path",
        EndpointInputLocation::Query => "Query",
        EndpointInputLocation::Header => "Header",
        EndpointInputLocation::Body => "Body",
    }
}

fn value_type_name(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Any => "任意结构",
        ValueType::Null => "空值",
        ValueType::Boolean => "布尔",
        ValueType::Integer => "整数",
        ValueType::Decimal => "小数",
        ValueType::Text => "文本",
        ValueType::TimestampMs => "时间戳",
        ValueType::File => "文件",
        ValueType::Object { .. } => "对象",
        ValueType::List { .. } => "列表",
        ValueType::Optional { .. } => "可选",
    }
}

