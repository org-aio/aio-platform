use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use dioxus::prelude::*;
use icons::{
    ArrowDown, ArrowUp, ArrowUpDown, ChevronLeft, ChevronRight, Eye, Pencil, Play, Plus, RefreshCw,
    Search, Sparkles, Trash2, X,
};
use serde_json::{Map, Value};

use crate::{
    CompiledModel, CompiledPage, CompiledPageEndpoint, CompiledPageRenderer, CompiledTable,
    CompiledTree, EndpointInputLocation, FieldRelation, FormStateExtractionRequest,
    FormStateExtractionResponse, MenuActionAccess, MenuRowActions, PageEndpointSource,
    ProgramImage, RestMethod, RuntimeRecordCriteria, RuntimeRecordFilter,
    RuntimeRecordFilterOperator, RuntimeRecordInput, RuntimeRecordPage, RuntimeRecordSort,
    RuntimeRecordSortDirection, RuntimeRecordView, SymbolId, ValueType,
    browser_http::{api_url, delete_api, get_api, patch_api, post_api},
    runtime_record_form::{
        record_payload_from_state, relation_form_state_value, relation_record_label,
        relation_search_fields, selected_relation_ids,
    },
    runtime_tree::RuntimeTree,
};
use az_ui_components::{
    button::{Button, ButtonSize, ButtonVariant},
    checkbox::{Checkbox, checkbox_is_checked, checkbox_state},
    data_table::{
        DataTable, DataTableAlign, DataTableCellContext, DataTableColumn, DataTableFixed,
        DataTableHeaderContext,
    },
    dialog::{Dialog, DialogDescription, DialogTitle},
    input::Input,
    textarea::Textarea,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ConventionPageContext {
    pub api_base_url: String,
    pub route: String,
    pub page: CompiledPage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInPageContext {
    pub api_base_url: String,
    pub image: ProgramImage,
    pub page: CompiledPage,
    pub row_actions: MenuRowActions,
}

#[component]
pub fn EndpointWorkbench(context: ConventionPageContext) -> Element {
    let mut generation = use_signal(|| 0_u64);
    let mut endpoint_dialog = use_signal(|| None::<CompiledPageEndpoint>);
    let page = context.page;
    let api_base_url = context.api_base_url;
    let query_endpoints = page
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.method == RestMethod::Get && endpoint.inputs.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let action_endpoints = page
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.method != RestMethod::Get || !endpoint.inputs.is_empty())
        .cloned()
        .collect::<Vec<_>>();

    rsx! {
        section { class: "aio-runtime-table-page aio-runtime-endpoint-workbench",
            header { class: "aio-runtime-table-page__header",
                div {
                    h2 { "{page.title}" }
                    p { "{query_endpoints.len()} 项数据 · {action_endpoints.len()} 项操作" }
                }
                div { class: "aio-runtime-table-page__actions",
                    for endpoint in action_endpoints {
                        {endpoint_action_button(endpoint, endpoint_dialog)}
                    }
                    Button {
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Outline,
                        title: "刷新数据",
                        aria_label: "刷新数据",
                        onclick: move |_| generation += 1,
                        RefreshCw { class: "size-4" }
                    }
                }
            }
            div { class: "aio-runtime-endpoint-workbench__content",
                if query_endpoints.is_empty() {
                    div { class: "aio-runtime-table-state", "暂无可读取的数据接口" }
                } else {
                    for endpoint in query_endpoints {
                        EndpointResultPanel {
                            key: "{endpoint.id}",
                            api_base_url: api_base_url.clone(),
                            endpoint,
                            generation,
                        }
                    }
                }
            }
            if let Some(endpoint) = endpoint_dialog() {
                RuntimeEndpointDialog {
                    key: "{endpoint.id}",
                    api_base_url: api_base_url.clone(),
                    endpoint,
                    on_close: move |_| endpoint_dialog.set(None),
                }
            }
        }
    }
}

#[component]
fn EndpointResultPanel(
    api_base_url: String,
    endpoint: CompiledPageEndpoint,
    generation: Signal<u64>,
) -> Element {
    let request_api = api_base_url;
    let request_endpoint = endpoint.clone();
    let response = use_resource(move || {
        let api_base_url = request_api.clone();
        let endpoint = request_endpoint.clone();
        let _generation = generation();
        async move { send_rest_endpoint_request(&api_base_url, &endpoint, &BTreeMap::new()).await }
    });
    let result = response.read().as_ref().cloned();

    rsx! {
        article { class: "aio-runtime-endpoint-panel",
            header {
                div {
                    strong { "{endpoint.title}" }
                    code { "{endpoint.method.as_str()} {endpoint.path}" }
                }
            }
            match result {
                None => rsx! { div { class: "aio-runtime-endpoint-panel__state", "正在加载" } },
                Some(Ok(payload)) => rsx! { pre { "{payload}" } },
                Some(Err(error)) => rsx! {
                    div { class: "aio-runtime-endpoint-panel__state is-error", role: "alert", "{error}" }
                },
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RecordDialog {
    Create,
    Detail(RuntimeRecordView),
    Edit(RuntimeRecordView),
    Delete(RuntimeRecordView),
}

type RelationLabelIndex = BTreeMap<SymbolId, BTreeMap<String, String>>;

#[component]
pub fn BuiltInPage(
    api_base_url: String,
    image: ProgramImage,
    page: CompiledPage,
    row_actions: MenuRowActions,
) -> Element {
    let (table, tree) = match &page.renderer {
        CompiledPageRenderer::TreeTable { tree, table, .. } => (table.clone(), Some(tree.clone())),
        CompiledPageRenderer::CrudTable { table, .. } => (table.clone(), None),
        CompiledPageRenderer::ConventionFile { .. } => {
            return render_runtime_error("内置页面收到了约定文件渲染计划");
        }
        CompiledPageRenderer::MenuTree { .. } => {
            return render_runtime_error("通用表格收到了菜单树渲染计划");
        }
    };
    let Some(model) = image.models.get(&table.model_id).cloned() else {
        return render_runtime_error("表格模型未进入 ProgramImage");
    };
    let generation = use_signal(|| 0_u64);
    let offset = use_signal(|| 0_usize);
    let filters = use_signal(BTreeMap::<SymbolId, String>::new);
    let sort = use_signal(|| None::<(SymbolId, bool)>);
    let selected_tree = use_signal(|| None::<String>);
    let dialog = use_signal(|| None::<RecordDialog>);
    let notice = use_signal(|| None::<String>);

    rsx! {
        MetadataTablePage {
            api_base_url,
            image,
            page,
            table,
            tree,
            model,
            row_actions,
            generation,
            offset,
            filters,
            sort,
            selected_tree,
            dialog,
            notice,
        }
    }
}

#[component]
fn MetadataTablePage(
    api_base_url: String,
    image: ProgramImage,
    page: CompiledPage,
    table: CompiledTable,
    tree: Option<CompiledTree>,
    model: CompiledModel,
    row_actions: MenuRowActions,
    generation: Signal<u64>,
    mut offset: Signal<usize>,
    mut filters: Signal<BTreeMap<SymbolId, String>>,
    mut sort: Signal<Option<(SymbolId, bool)>>,
    mut selected_tree: Signal<Option<String>>,
    mut dialog: Signal<Option<RecordDialog>>,
    mut notice: Signal<Option<String>>,
) -> Element {
    let mut endpoint_dialog = use_signal(|| None::<CompiledPageEndpoint>);
    let page_size = table.page_size as usize;
    let records_api = api_base_url.clone();
    let records_model = table.model_id;
    let records_model_metadata = model.clone();
    let relation_field_name = tree.as_ref().and_then(|tree| {
        compiled_field(&model, tree.table_relation_field_id).map(|(name, _, _)| name.to_owned())
    });
    use_effect(move || {
        let _selected = selected_tree();
        offset.set(0);
    });
    let records = use_resource(move || {
        let api_base_url = records_api.clone();
        let current_offset = offset();
        let selected_tree = selected_tree();
        let current_filters = filters();
        let current_sort = sort();
        let relation_field_name = relation_field_name.clone();
        let model = records_model_metadata.clone();
        let _generation = generation();
        async move {
            let criteria = runtime_table_criteria(
                &model,
                &current_filters,
                current_sort,
                relation_field_name.as_deref(),
                selected_tree.as_deref(),
            )?;
            let path = runtime_records_path(records_model, current_offset, page_size, &criteria)?;
            get_api::<RuntimeRecordPage>(&api_base_url, &path).await
        }
    });
    let tree_records_api = api_base_url.clone();
    let tree_model = tree.as_ref().map(|tree| tree.model_id);
    let tree_records = use_resource(move || {
        let api_base_url = tree_records_api.clone();
        async move {
            let Some(model_id) = tree_model else {
                return Ok(None);
            };
            get_api::<RuntimeRecordPage>(
                &api_base_url,
                &format!("/api/runtime/models/{model_id}/records?o=0&s=200"),
            )
            .await
            .map(Some)
        }
    });
    let relation_records_api = api_base_url.clone();
    let relation_image = image.clone();
    let relation_model = model.clone();
    let relation_source = records;
    let relation_labels = use_resource(move || {
        let api_base_url = relation_records_api.clone();
        let image = relation_image.clone();
        let model = relation_model.clone();
        let record_page = relation_source.read().as_ref().cloned();
        async move {
            let Some(Ok(page)) = record_page else {
                return Ok(RelationLabelIndex::new());
            };
            let references = relation_reference_ids(&model, &page);
            load_relation_label_index(&api_base_url, &image, &references).await
        }
    });
    let record_page = records.read().as_ref().cloned();
    let tree_page = tree_records.read().as_ref().cloned();
    let relation_label_result = relation_labels.read().as_ref().cloned();
    let relation_label_index = relation_label_result
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .map_or_else(RelationLabelIndex::new, |index| index);
    let relation_label_error = relation_label_result
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .cloned();
    let field_columns = table_columns(&model);
    let data_table_columns = runtime_table_columns(&model, &field_columns);
    let filter_fields = filter_fields(&model);
    let can_create = !matches!(row_actions.edit, MenuActionAccess::Hidden);
    let rows = record_page
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|page| page.d.clone())
        .unwrap_or_default();
    let total = record_page
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|page| page.t)
        .unwrap_or_default();
    let has_previous = offset() > 0;
    let has_next = offset().saturating_add(page_size) < total as usize;
    let empty_text = match record_page.as_ref() {
        Some(Err(error)) => error.clone(),
        None => "正在加载".to_owned(),
        Some(Ok(_)) => "暂无数据".to_owned(),
    };
    let external_endpoints = page
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.source != PageEndpointSource::BuiltIn)
        .cloned()
        .collect::<Vec<_>>();
    rsx! {
        section { class: "aio-runtime-table-page",
            header { class: "aio-runtime-table-page__header",
                div {
                    h2 { "{page.title}" }
                    p { "{model.title}" }
                }
                div { class: "aio-runtime-table-page__actions",
                    for endpoint in external_endpoints {
                        {endpoint_action_button(endpoint, endpoint_dialog)}
                    }
                    if can_create {
                        Button { onclick: move |_| dialog.set(Some(RecordDialog::Create)),
                            Plus { class: "size-4" }
                            "新增"
                        }
                    }
                }
            }
            if let Some(message) = notice() {
                div { class: "aio-runtime-notice", "{message}" }
            }
            div { class: if tree.is_some() { "aio-runtime-table-page__tree-layout" } else { "aio-runtime-table-page__content" },
                if let Some(tree) = tree.as_ref() {
                    aside { class: "aio-runtime-tree",
                        strong { "分类" }
                        match tree_page.as_ref() {
                            Some(Ok(Some(tree_page))) => rsx! {
                                RuntimeTree {
                                    tree: tree.clone(),
                                    image: image.clone(),
                                    page: tree_page.clone(),
                                    selected_record: selected_tree,
                                }
                            },
                            Some(Err(error)) => rsx! {
                                div { class: "aio-runtime-table-state is-error", "{error}" }
                            },
                            None => rsx! {
                                div { class: "aio-runtime-table-state", "正在加载分类" }
                            },
                            Some(Ok(None)) => rsx! {},
                        }
                    }
                }
                div { class: "aio-runtime-table-page__content",
                    if !filter_fields.is_empty() {
                        form { class: "aio-runtime-filters", onsubmit: move |event| {
                            event.prevent_default();
                            filters.set(filter_fields.iter().filter_map(|field_id| {
                                let value = form_text(&event, &field_id.to_string());
                                (!value.trim().is_empty()).then_some((*field_id, value))
                            }).collect());
                            offset.set(0);
                        },
                            for field_id in &filter_fields {
                                if let Some((_, title, _)) = compiled_field(&model, *field_id) {
                                    Input { class: "aio-input", name: "{field_id}", placeholder: "{title}" }
                                }
                            }
                            Button { r#type: "submit",
                                Search { class: "size-4" }
                                "查询"
                            }
                            Button {
                                r#type: "reset",
                                variant: ButtonVariant::Outline,
                                onclick: move |_| {
                                    filters.set(BTreeMap::new());
                                    offset.set(0);
                                },
                                "重置"
                            }
                        }
                    }
                    if let Some(error) = relation_label_error {
                        div { class: "aio-runtime-table-state is-error", role: "alert",
                            "关联记录标签加载失败：{error}"
                        }
                    }
                    DataTable::<RuntimeRecordView> {
                        class: "aio-runtime-data-table",
                        aria_label: format!("{}数据表", model.title),
                        rows,
                        columns: data_table_columns,
                        max_height: "calc(100vh - 19rem)",
                        empty_text,
                        row_key: |record: RuntimeRecordView| record.id.clone(),
                        render_header: {
                            let model = model.clone();
                            move |header: DataTableHeaderContext| {
                                runtime_table_header(header, &model, sort, offset)
                            }
                        },
                        render_cell: {
                            let model = model.clone();
                            let relation_label_index = relation_label_index.clone();
                            move |cell: DataTableCellContext<RuntimeRecordView>| {
                                runtime_table_cell(
                                    cell,
                                    &model,
                                    &relation_label_index,
                                    offset(),
                                    row_actions.clone(),
                                    dialog,
                                )
                            }
                        },
                    }
                    footer { class: "aio-runtime-pagination",
                        span { "共 {total} 条" }
                        Button {
                            size: ButtonSize::IconSm,
                            variant: ButtonVariant::Outline,
                            disabled: !has_previous,
                            aria_label: "上一页",
                            onclick: move |_| offset.set(offset().saturating_sub(page_size)),
                            ChevronLeft { class: "size-4" }
                        }
                        span { "第 {offset() / page_size + 1} 页" }
                        Button {
                            size: ButtonSize::IconSm,
                            variant: ButtonVariant::Outline,
                            disabled: !has_next,
                            aria_label: "下一页",
                            onclick: move |_| offset.set(offset().saturating_add(page_size)),
                            ChevronRight { class: "size-4" }
                        }
                    }
                }
            }
            if let Some(dialog_value) = dialog() {
                RuntimeRecordDialog {
                    key: "{table.model_id}:{dialog_key(&dialog_value)}",
                    value: dialog_value,
                    model,
                    image: image.clone(),
                    api_base_url: api_base_url.clone(),
                    model_id: table.model_id,
                    generation,
                    dialog,
                    notice,
                }
            }
            if let Some(endpoint) = endpoint_dialog() {
                RuntimeEndpointDialog {
                    key: "{endpoint.id}",
                    api_base_url: api_base_url.clone(),
                    endpoint,
                    on_close: move |_| endpoint_dialog.set(None),
                }
            }
        }
    }
}

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
                        {rest_endpoint_input(input)}
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

fn rest_endpoint_input(input: &crate::CompiledEndpointInput) -> Element {
    let input_type = match input.value_type {
        ValueType::Integer | ValueType::Decimal | ValueType::TimestampMs => "number",
        ValueType::File => "file",
        _ => "text",
    };
    if input.value_type == ValueType::Boolean {
        return rsx! {
            select { name: "{input.name}", class: "aio-input", required: input.required,
                option { value: "", "选择" }
                option { value: "true", "是" }
                option { value: "false", "否" }
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

#[component]
fn RuntimeRecordDialog(
    value: RecordDialog,
    model: CompiledModel,
    image: ProgramImage,
    api_base_url: String,
    model_id: SymbolId,
    generation: Signal<u64>,
    mut dialog: Signal<Option<RecordDialog>>,
    notice: Signal<Option<String>>,
) -> Element {
    let title = match &value {
        RecordDialog::Create => "新增记录",
        RecordDialog::Detail(_) => "记录详情",
        RecordDialog::Edit(_) => "编辑记录",
        RecordDialog::Delete(_) => "确认删除",
    };
    let record = match &value {
        RecordDialog::Detail(record)
        | RecordDialog::Edit(record)
        | RecordDialog::Delete(record) => Some(record.clone()),
        RecordDialog::Create => None,
    };
    let readonly = matches!(value, RecordDialog::Detail(_));
    let deleting = matches!(value, RecordDialog::Delete(_));
    let dialog_class = if deleting {
        "aio-runtime-dialog aio-runtime-dialog--confirm"
    } else {
        "aio-runtime-dialog aio-runtime-dialog--record"
    };
    let description = record.as_ref().map_or_else(
        || format!("为“{}”填写记录字段", model.title),
        |record| format!("{} · {}", model.title, record.id),
    );
    let submit_value = value.clone();
    let initial_form_state = initial_form_state(&model, record.as_ref());
    let form_state = use_signal(move || initial_form_state);
    let mut ai_prompt = use_signal(String::new);
    let ai_loading = use_signal(|| false);
    let can_ai_fill = !readonly
        && model
            .field_options
            .values()
            .any(|options| options.form_visible && options.form_editable && options.ai_extract);
    let submit_model = model.clone();
    rsx! {
        Dialog {
            class: dialog_class,
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    dialog.set(None);
                }
            },
            header { class: "aio-runtime-dialog__header",
                div { class: "aio-runtime-dialog__heading",
                    DialogTitle { "{title}" }
                    DialogDescription { "{description}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭记录对话框",
                    aria_label: "关闭记录对话框",
                    onclick: move |_| dialog.set(None),
                    X { class: "size-4" }
                }
            }
            if deleting {
                p { class: "aio-runtime-dialog__confirm-message", "删除后不可恢复，确认删除这条记录？" }
                footer { class: "aio-runtime-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| dialog.set(None),
                        "取消"
                    }
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Destructive,
                        onclick: move |_| {
                            if let Some(record) = record.clone() {
                                delete_runtime_record(
                                    api_base_url.clone(), model_id,
                                    record.id, generation, dialog, notice,
                                );
                            }
                        },
                        Trash2 { class: "size-4" }
                        "删除记录"
                    }
                }
            } else {
                form { class: "aio-runtime-record-form", onsubmit: move |event| {
                    event.prevent_default();
                    if readonly {
                        dialog.set(None);
                        return;
                    }
                    match record_payload_from_state(&submit_model, &form_state()) {
                        Ok(payload) => save_runtime_record(
                            api_base_url.clone(), model_id,
                            submit_value.clone(), payload, generation, dialog, notice,
                        ),
                        Err(error) => notice.set(Some(error)),
                    }
                },
                    div { class: "aio-runtime-record-form__body",
                        if can_ai_fill {
                            div { class: "aio-runtime-ai-fill",
                                Textarea {
                                    class: "aio-input",
                                    aria_label: "AI 表单输入",
                                    placeholder: "描述要填写的数据",
                                    value: ai_prompt(),
                                    oninput: move |event: FormEvent| ai_prompt.set(event.value()),
                                }
                                Button {
                                    r#type: "button",
                                    variant: ButtonVariant::Outline,
                                    disabled: ai_loading(),
                                    onclick: {
                                        let api_base_url = api_base_url.clone();
                                        let model = model.clone();
                                        move |_| {
                                            let prompt = ai_prompt().trim().to_owned();
                                            if prompt.is_empty() {
                                                notice.set(Some("AI 表单输入不能为空".to_owned()));
                                                return;
                                            }
                                            extract_runtime_form_state(
                                                api_base_url.clone(),
                                                model_id,
                                                model.clone(),
                                                prompt,
                                                form_state,
                                                ai_loading,
                                                notice,
                                            );
                                        }
                                    },
                                    Sparkles { class: "size-4" }
                                    if ai_loading() { "生成中" } else { "AI 填写" }
                                }
                            }
                        }
                        for slot in 0..model.field_names.len() as u32 {
                            RuntimeRecordField {
                                key: "{model.id}:{slot}",
                                slot,
                                model: model.clone(),
                                image: image.clone(),
                                api_base_url: api_base_url.clone(),
                                readonly,
                                form_state,
                            }
                        }
                    }
                    footer { class: "aio-runtime-dialog__actions",
                        if !readonly {
                            Button {
                                r#type: "button",
                                variant: ButtonVariant::Ghost,
                                onclick: move |_| dialog.set(None),
                                "取消"
                            }
                        }
                        Button { r#type: "submit", if readonly { "关闭" } else { "保存记录" } }
                    }
                }
            }
        }
    }
}

#[component]
fn RuntimeRecordField(
    slot: u32,
    model: CompiledModel,
    image: ProgramImage,
    api_base_url: String,
    readonly: bool,
    form_state: Signal<BTreeMap<String, String>>,
) -> Element {
    let (Some(name), Some(title), Some(value_type), Some(options)) = (
        model.field_names.get(&slot),
        model.field_titles.get(&slot),
        model.field_types.get(&slot),
        model.field_options.get(&slot),
    ) else {
        return rsx! {};
    };
    let visible = if readonly {
        options.detail_visible
    } else {
        options.form_visible
    };
    if !visible {
        return rsx! {};
    }
    let input_id = format!("runtime-record-{}-{slot}", model.id);
    let disabled = readonly || !options.form_editable;
    let required = model.required_fields.contains(&slot);
    let value = form_state()
        .get(name)
        .cloned()
        .map_or_else(String::new, |value| value);
    let field_class = if model.field_relations.contains_key(&slot)
        || matches!(
            value_type,
            ValueType::Object { .. } | ValueType::List { .. }
        ) {
        "aio-runtime-record-form__field aio-runtime-record-form__field--wide"
    } else {
        "aio-runtime-record-form__field"
    };
    rsx! {
        div { class: field_class,
            label { r#for: "{input_id}", "{title}" }
            if let Some(relation) = model.field_relations.get(&slot) {
                if let Some(target_model) = image.models.get(&relation.target_model_id) {
                    RuntimeRelationField {
                        api_base_url,
                        relation: relation.clone(),
                        target_model: target_model.clone(),
                        input_id: input_id.clone(),
                        field_name: name.clone(),
                        field_title: title.clone(),
                        required,
                        disabled,
                        form_state,
                    }
                } else {
                    div { class: "aio-runtime-relation-state is-error", role: "alert",
                        "关联模型未进入运行时 Image"
                    }
                }
            } else if matches!(value_type, ValueType::Boolean) {
                Checkbox {
                    id: input_id.clone(),
                    name: "{name}",
                    disabled,
                    checked: Some(checkbox_state(matches!(value.as_str(), "true" | "on" | "1"))),
                    on_checked_change: {
                        let name = name.clone();
                        move |checked| form_state.with_mut(|state| {
                            state.insert(name.clone(), checkbox_is_checked(checked).to_string());
                        })
                    },
                }
            } else {
                Input {
                    id: input_id.clone(),
                    class: "aio-input",
                    name: "{name}",
                    r#type: field_input_type(value_type),
                    required,
                    readonly: disabled,
                    placeholder: options.placeholder.as_deref().map_or("", |value| value),
                    value,
                    oninput: {
                        let name = name.clone();
                        move |event: FormEvent| form_state.with_mut(|state| {
                            state.insert(name.clone(), event.value());
                        })
                    },
                }
            }
            if let Some(help_text) = options.help_text.as_deref() {
                small { "{help_text}" }
            }
        }
    }
}

#[component]
fn RuntimeRelationField(
    api_base_url: String,
    relation: FieldRelation,
    target_model: CompiledModel,
    input_id: String,
    field_name: String,
    field_title: String,
    required: bool,
    disabled: bool,
    mut form_state: Signal<BTreeMap<String, String>>,
) -> Element {
    let mut search_draft = use_signal(String::new);
    let mut search_term = use_signal(String::new);
    let mut page_offset = use_signal(|| 0_usize);
    let page_size = 20_usize;
    let search_fields = relation_search_fields(&target_model);
    let records_api = api_base_url.clone();
    let target_model_id = relation.target_model_id;
    let records_search_fields = search_fields.clone();
    let records = use_resource(move || {
        let api_base_url = records_api.clone();
        let term = search_term();
        let offset = page_offset();
        let search_fields = records_search_fields.clone();
        async move {
            let criteria = RuntimeRecordCriteria {
                all: Vec::new(),
                any: if term.is_empty() {
                    Vec::new()
                } else {
                    search_fields
                        .iter()
                        .map(|field| RuntimeRecordFilter {
                            field: field.clone(),
                            operator: RuntimeRecordFilterOperator::Contains,
                            value: term.clone(),
                        })
                        .collect()
                },
                sort: search_fields.first().map(|field| RuntimeRecordSort {
                    field: field.clone(),
                    direction: RuntimeRecordSortDirection::Ascending,
                }),
            };
            let path = runtime_records_path(target_model_id, offset, page_size, &criteria)?;
            get_api::<RuntimeRecordPage>(&api_base_url, &path).await
        }
    });
    let selected_field_name = field_name.clone();
    let selected_value = use_memo(move || {
        form_state()
            .get(&selected_field_name)
            .cloned()
            .map_or_else(String::new, |value| value)
    });
    let selected_api = api_base_url;
    let selected_relation = relation.clone();
    let selected_title = field_title.clone();
    let selected_records = use_resource(move || {
        let api_base_url = selected_api.clone();
        let current = selected_value();
        let relation = selected_relation.clone();
        let title = selected_title.clone();
        async move {
            let ids = selected_relation_ids(&relation, &current, &title)?;
            let mut records = Vec::new();
            for id in ids {
                let path = format!("/api/runtime/models/{target_model_id}/records/{id}");
                records.push(get_api::<RuntimeRecordView>(&api_base_url, &path).await?);
            }
            Ok::<_, String>(records)
        }
    });
    let current = selected_value();
    let (selected_ids, selection_error) =
        match selected_relation_ids(&relation, &current, &field_title) {
            Ok(ids) => (ids, None),
            Err(error) => (Vec::new(), Some(error)),
        };
    let record_page = records.read().as_ref().cloned();
    let (mut rows, total, loading, load_error) = match record_page {
        Some(Ok(page)) => (page.d, page.t, false, None),
        Some(Err(error)) => (Vec::new(), 0, false, Some(error)),
        None => (Vec::new(), 0, true, None),
    };
    let selected_record_result = selected_records.read().as_ref().cloned();
    let selected_load_error = selected_record_result
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .cloned();
    if let Some(Ok(selected)) = selected_record_result {
        for record in selected.into_iter().rev() {
            if !rows.iter().any(|candidate| candidate.id == record.id) {
                rows.insert(0, record);
            }
        }
    }
    let select_disabled = disabled || loading || load_error.is_some() || rows.is_empty();
    let has_previous = page_offset() > 0;
    let has_next = page_offset().saturating_add(page_size) < total as usize;
    rsx! {
        div { class: "aio-runtime-relation-picker",
            if !disabled && !search_fields.is_empty() {
                div { class: "aio-runtime-relation-search",
                    Input {
                        class: "aio-input",
                        aria_label: "搜索{target_model.title}",
                        placeholder: "搜索{target_model.title}",
                        value: search_draft(),
                        oninput: move |event: FormEvent| search_draft.set(event.value()),
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Outline,
                        title: "搜索{target_model.title}",
                        aria_label: "搜索{target_model.title}",
                        onclick: move |_| {
                            search_term.set(search_draft().trim().to_owned());
                            page_offset.set(0);
                        },
                        Search { class: "size-4" }
                    }
                    if !search_term().is_empty() {
                        Button {
                            r#type: "button",
                            size: ButtonSize::IconSm,
                            variant: ButtonVariant::Ghost,
                            title: "清除{target_model.title}搜索",
                            aria_label: "清除{target_model.title}搜索",
                            onclick: move |_| {
                                search_draft.set(String::new());
                                search_term.set(String::new());
                                page_offset.set(0);
                            },
                            X { class: "size-4" }
                        }
                    }
                }
            }
            if let Some(error) = selection_error {
                div { class: "aio-runtime-relation-state is-error", role: "alert", "{error}" }
            }
            if let Some(error) = selected_load_error {
                div { class: "aio-runtime-relation-state is-error", role: "alert", "{error}" }
            }
            if loading {
                div { class: "aio-runtime-relation-state", "正在加载{target_model.title}" }
            } else if let Some(error) = load_error {
                div { class: "aio-runtime-relation-state is-error", role: "alert", "{error}" }
            } else if rows.is_empty() {
                div { class: "aio-runtime-relation-state", "暂无{target_model.title}，请先创建关联记录" }
            }
            if relation.kind.is_collection() {
                select {
                    id: input_id,
                    class: "aio-input aio-runtime-relation-select is-multiple",
                    name: field_name.clone(),
                    aria_label: field_title,
                    multiple: true,
                    required,
                    disabled: select_disabled,
                    onchange: move |event: FormEvent| {
                        let ids = relation_event_values(&event);
                        let value = relation_form_state_value(relation.kind, ids);
                        form_state.with_mut(|state| {
                            state.insert(field_name.clone(), value);
                        });
                    },
                    for record in &rows {
                        option {
                            value: record.id.clone(),
                            selected: selected_ids.contains(&record.id),
                            "{relation_record_label(&target_model, record)}"
                        }
                    }
                }
            } else {
                select {
                    id: input_id,
                    class: "aio-input aio-runtime-relation-select",
                    name: field_name.clone(),
                    aria_label: field_title,
                    required,
                    disabled: select_disabled,
                    onchange: move |event: FormEvent| {
                        let ids = relation_event_values(&event);
                        let value = relation_form_state_value(relation.kind, ids);
                        form_state.with_mut(|state| {
                            state.insert(field_name.clone(), value);
                        });
                    },
                    option { value: "", selected: selected_ids.is_empty(), "请选择{target_model.title}" }
                    for record in &rows {
                        option {
                            value: record.id.clone(),
                            selected: selected_ids.contains(&record.id),
                            "{relation_record_label(&target_model, record)}"
                        }
                    }
                }
            }
            if !disabled && total > 0 {
                footer { class: "aio-runtime-relation-pagination",
                    span { "共 {total} 条" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        disabled: !has_previous,
                        title: "上一页{target_model.title}",
                        aria_label: "上一页{target_model.title}",
                        onclick: move |_| page_offset.set(page_offset().saturating_sub(page_size)),
                        ChevronLeft { class: "size-4" }
                    }
                    span { "第 {page_offset() / page_size + 1} 页" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::IconSm,
                        variant: ButtonVariant::Ghost,
                        disabled: !has_next,
                        title: "下一页{target_model.title}",
                        aria_label: "下一页{target_model.title}",
                        onclick: move |_| page_offset.set(page_offset().saturating_add(page_size)),
                        ChevronRight { class: "size-4" }
                    }
                }
            }
        }
    }
}

fn relation_event_values(event: &FormEvent) -> Vec<String> {
    event
        .values()
        .into_iter()
        .filter_map(|(_, value)| match value {
            dioxus::html::FormValue::Text(value) if !value.trim().is_empty() => Some(value),
            _ => None,
        })
        .collect()
}

fn dialog_key(value: &RecordDialog) -> String {
    match value {
        RecordDialog::Create => "create".to_owned(),
        RecordDialog::Detail(record) => format!("detail:{}", record.id),
        RecordDialog::Edit(record) => format!("edit:{}", record.id),
        RecordDialog::Delete(record) => format!("delete:{}", record.id),
    }
}

fn initial_form_state(
    model: &CompiledModel,
    record: Option<&RuntimeRecordView>,
) -> BTreeMap<String, String> {
    model
        .field_names
        .iter()
        .map(|(slot, name)| {
            let value = record
                .and_then(|record| record.payload.get(name))
                .or_else(|| {
                    model
                        .field_options
                        .get(slot)
                        .and_then(|options| options.default_value.as_ref())
                })
                .map(value_to_text)
                .unwrap_or_default();
            (name.clone(), value)
        })
        .collect()
}

fn extract_runtime_form_state(
    api_base_url: String,
    model_id: SymbolId,
    model: CompiledModel,
    prompt: String,
    mut form_state: Signal<BTreeMap<String, String>>,
    mut ai_loading: Signal<bool>,
    mut notice: Signal<Option<String>>,
) {
    let current_form_state = match record_payload_from_state(&model, &form_state()) {
        Ok(value) => value,
        Err(error) => {
            notice.set(Some(error));
            return;
        }
    };
    ai_loading.set(true);
    spawn(async move {
        let input = FormStateExtractionRequest {
            prompt,
            current_form_state,
            model: None,
        };
        let path = format!("/api/runtime/models/{model_id}/form-state/extract");
        match post_api::<_, FormStateExtractionResponse>(&api_base_url, &path, &input).await {
            Ok(response) => {
                if let Some(values) = response.form_state.as_object() {
                    form_state.with_mut(|state| {
                        for (name, value) in values {
                            state.insert(name.clone(), value_to_text(value));
                        }
                    });
                    notice.set(Some(format!("AI 已填写表单 · {}", response.model)));
                } else {
                    notice.set(Some("AI 返回的 formState 不是对象".to_owned()));
                }
            }
            Err(error) => notice.set(Some(error)),
        }
        ai_loading.set(false);
    });
}

fn save_runtime_record(
    api_base_url: String,
    model_id: SymbolId,
    dialog_value: RecordDialog,
    payload: Value,
    mut generation: Signal<u64>,
    mut dialog: Signal<Option<RecordDialog>>,
    mut notice: Signal<Option<String>>,
) {
    spawn(async move {
        let base = format!("/api/runtime/models/{model_id}/records");
        let input = RuntimeRecordInput { payload };
        let result = match dialog_value {
            RecordDialog::Create => {
                post_api::<_, RuntimeRecordView>(&api_base_url, &base, &input).await
            }
            RecordDialog::Edit(record) => {
                patch_api::<_, RuntimeRecordView>(
                    &api_base_url,
                    &format!("{base}/{}", record.id),
                    &input,
                )
                .await
            }
            RecordDialog::Detail(_) | RecordDialog::Delete(_) => return,
        };
        match result {
            Ok(_) => {
                dialog.set(None);
                notice.set(Some("记录已保存".to_owned()));
                generation.with_mut(|value| *value = value.saturating_add(1));
            }
            Err(error) => notice.set(Some(error)),
        }
    });
}

fn delete_runtime_record(
    api_base_url: String,
    model_id: SymbolId,
    record_id: String,
    mut generation: Signal<u64>,
    mut dialog: Signal<Option<RecordDialog>>,
    mut notice: Signal<Option<String>>,
) {
    spawn(async move {
        let path = format!("/api/runtime/models/{model_id}/records/{record_id}");
        match delete_api::<bool>(&api_base_url, &path).await {
            Ok(_) => {
                dialog.set(None);
                notice.set(Some("记录已删除".to_owned()));
                generation.with_mut(|value| *value = value.saturating_add(1));
            }
            Err(error) => notice.set(Some(error)),
        }
    });
}

fn field_input_type(value_type: &ValueType) -> &'static str {
    match value_type {
        ValueType::Boolean => "checkbox",
        ValueType::Integer | ValueType::Decimal | ValueType::TimestampMs => "number",
        _ => "text",
    }
}

fn table_columns(model: &CompiledModel) -> Vec<SymbolId> {
    model
        .field_slots
        .iter()
        .filter_map(|(field_id, slot)| {
            model
                .field_options
                .get(slot)
                .is_some_and(|options| options.list_visible)
                .then_some(*field_id)
        })
        .collect()
}

fn runtime_table_columns(
    model: &CompiledModel,
    field_columns: &[SymbolId],
) -> Vec<DataTableColumn> {
    let fields = field_columns
        .iter()
        .filter_map(|field_id| {
            let (_, title, value_type) = compiled_field(model, *field_id)?;
            let width = match value_type {
                ValueType::Boolean => 96,
                ValueType::Integer | ValueType::Decimal | ValueType::TimestampMs => 128,
                _ => 180,
            };
            Some(
                DataTableColumn::leaf(format!("field:{field_id}"), title)
                    .width(width)
                    .align(DataTableAlign::Start),
            )
        })
        .collect::<Vec<_>>();
    let mut columns = vec![
        DataTableColumn::leaf("index", "序号")
            .width(72)
            .align(DataTableAlign::Center)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("id", "ID")
            .width(match model.primary_key.generation {
                crate::PrimaryKeyGeneration::Uuid => 280,
                crate::PrimaryKeyGeneration::AutoIncrement => 120,
            })
            .fixed(DataTableFixed::Left),
    ];
    if !fields.is_empty() {
        columns.push(DataTableColumn::group(
            "fields",
            model.title.clone(),
            fields,
        ));
    }
    columns.push(
        DataTableColumn::leaf("actions", "操作")
            .width(120)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    );
    columns
}

fn runtime_table_header(
    header: DataTableHeaderContext,
    model: &CompiledModel,
    mut sort: Signal<Option<(SymbolId, bool)>>,
    mut offset: Signal<usize>,
) -> Element {
    let Some(field_id) = header
        .column
        .key
        .strip_prefix("field:")
        .and_then(|value| SymbolId::parse(value).ok())
    else {
        return rsx! { "{header.column.title}" };
    };
    let sortable = model
        .field_slots
        .get(&field_id)
        .and_then(|slot| model.field_options.get(slot))
        .is_some_and(|options| options.sortable);
    if !sortable {
        return rsx! { "{header.column.title}" };
    }
    let title = header.column.title;
    rsx! {
        Button {
            class: "aio-runtime-sort",
            title: "按 {title} 排序",
            onclick: move |_| {
                sort.set(match sort() {
                    Some((current, ascending)) if current == field_id => {
                        Some((field_id, !ascending))
                    }
                    _ => Some((field_id, true)),
                });
                offset.set(0);
            },
            "{title}"
            match sort() {
                Some((current, true)) if current == field_id => rsx! { ArrowUp { class: "size-3" } },
                Some((current, false)) if current == field_id => rsx! { ArrowDown { class: "size-3" } },
                _ => rsx! { ArrowUpDown { class: "size-3" } },
            }
        }
    }
}

fn runtime_table_cell(
    cell: DataTableCellContext<RuntimeRecordView>,
    model: &CompiledModel,
    relation_labels: &RelationLabelIndex,
    offset: usize,
    row_actions: MenuRowActions,
    mut dialog: Signal<Option<RecordDialog>>,
) -> Element {
    if cell.column.key == "index" {
        return rsx! { "{offset + cell.row_index + 1}" };
    }
    if cell.column.key == "id" {
        return rsx! { code { "{cell.row.id}" } };
    }
    if cell.column.key == "actions" {
        let detail_record = cell.row.clone();
        let edit_record = cell.row.clone();
        let delete_record = cell.row;
        return rsx! {
            div { class: "aio-runtime-row-actions",
                if !matches!(row_actions.detail, MenuActionAccess::Hidden) {
                    Button {
                        title: "详情",
                        aria_label: "详情",
                        onclick: move |_| dialog.set(Some(RecordDialog::Detail(detail_record.clone()))),
                        Eye { class: "size-4" }
                    }
                }
                if !matches!(row_actions.edit, MenuActionAccess::Hidden) {
                    Button {
                        title: "编辑",
                        aria_label: "编辑",
                        onclick: move |_| dialog.set(Some(RecordDialog::Edit(edit_record.clone()))),
                        Pencil { class: "size-4" }
                    }
                }
                if !matches!(row_actions.delete, MenuActionAccess::Hidden) {
                    Button {
                        class: "is-destructive",
                        title: "删除",
                        aria_label: "删除",
                        onclick: move |_| dialog.set(Some(RecordDialog::Delete(delete_record.clone()))),
                        Trash2 { class: "size-4" }
                    }
                }
            }
        };
    }
    let field_id = cell
        .column
        .key
        .strip_prefix("field:")
        .and_then(|value| SymbolId::parse(value).ok());
    let value = field_id
        .and_then(|field_id| {
            record_field(&cell.row, model, field_id)
                .map(|value| runtime_field_value_to_text(model, field_id, value, relation_labels))
        })
        .unwrap_or_else(|| "—".to_owned());
    rsx! { "{value}" }
}

async fn load_relation_label_index(
    api_base_url: &str,
    image: &ProgramImage,
    references: &BTreeMap<SymbolId, BTreeSet<String>>,
) -> std::result::Result<RelationLabelIndex, String> {
    let mut index = BTreeMap::new();
    for (model_id, record_ids) in references {
        let target_model = image
            .models
            .get(model_id)
            .ok_or_else(|| format!("关联模型未进入运行时 Image: {model_id}"))?;
        let mut labels = BTreeMap::new();
        for record_id in record_ids {
            let path = format!("/api/runtime/models/{model_id}/records/{record_id}");
            let record = get_api::<RuntimeRecordView>(api_base_url, &path).await?;
            labels.insert(
                record.id.clone(),
                relation_record_label(target_model, &record),
            );
        }
        index.insert(*model_id, labels);
    }
    Ok(index)
}

fn relation_reference_ids(
    model: &CompiledModel,
    page: &RuntimeRecordPage,
) -> BTreeMap<SymbolId, BTreeSet<String>> {
    let mut references = BTreeMap::<SymbolId, BTreeSet<String>>::new();
    for (slot, relation) in &model.field_relations {
        if !model
            .field_options
            .get(slot)
            .is_some_and(|options| options.list_visible)
        {
            continue;
        }
        let Some(field) = model.field_names.get(slot) else {
            continue;
        };
        let record_ids = references.entry(relation.target_model_id).or_default();
        for value in page.d.iter().filter_map(|record| record.payload.get(field)) {
            if relation.kind.is_collection() {
                if let Value::Array(ids) = value {
                    record_ids.extend(ids.iter().filter_map(Value::as_str).map(str::to_owned));
                }
            } else if let Some(id) = value.as_str() {
                record_ids.insert(id.to_owned());
            }
        }
    }
    references
}

fn runtime_field_value_to_text(
    model: &CompiledModel,
    field_id: SymbolId,
    value: &Value,
    relation_labels: &RelationLabelIndex,
) -> String {
    let Some(slot) = model.field_slots.get(&field_id) else {
        return value_to_text(value);
    };
    let Some(relation) = model.field_relations.get(slot) else {
        return value_to_text(value);
    };
    let Some(labels) = relation_labels.get(&relation.target_model_id) else {
        return value_to_text(value);
    };
    if relation.kind.is_collection() {
        let Value::Array(ids) = value else {
            return value_to_text(value);
        };
        return ids
            .iter()
            .filter_map(Value::as_str)
            .map(|id| labels.get(id).map_or(id, String::as_str))
            .collect::<Vec<_>>()
            .join("、");
    }
    value
        .as_str()
        .map(|id| labels.get(id).map_or(id, String::as_str).to_owned())
        .unwrap_or_else(|| value_to_text(value))
}

fn filter_fields(model: &CompiledModel) -> Vec<SymbolId> {
    model
        .field_slots
        .iter()
        .filter_map(|(field_id, slot)| {
            model
                .field_options
                .get(slot)
                .is_some_and(|options| options.filterable)
                .then_some(*field_id)
        })
        .collect()
}

fn runtime_table_criteria(
    model: &CompiledModel,
    filters: &BTreeMap<SymbolId, String>,
    sort: Option<(SymbolId, bool)>,
    relation_field_name: Option<&str>,
    selected_tree: Option<&str>,
) -> std::result::Result<RuntimeRecordCriteria, String> {
    let mut all = Vec::new();
    for (field_id, value) in filters {
        let (field, _, _) = compiled_field(model, *field_id)
            .ok_or_else(|| format!("筛选字段未进入编译模型: {field_id}"))?;
        all.push(RuntimeRecordFilter {
            field: field.to_owned(),
            operator: RuntimeRecordFilterOperator::Contains,
            value: value.clone(),
        });
    }
    if let (Some(field), Some(value)) = (relation_field_name, selected_tree) {
        all.push(RuntimeRecordFilter {
            field: field.to_owned(),
            operator: RuntimeRecordFilterOperator::Equals,
            value: value.to_owned(),
        });
    }
    let sort = match sort {
        Some((field_id, ascending)) => {
            let (field, _, _) = compiled_field(model, field_id)
                .ok_or_else(|| format!("排序字段未进入编译模型: {field_id}"))?;
            Some(RuntimeRecordSort {
                field: field.to_owned(),
                direction: if ascending {
                    RuntimeRecordSortDirection::Ascending
                } else {
                    RuntimeRecordSortDirection::Descending
                },
            })
        }
        None => None,
    };
    Ok(RuntimeRecordCriteria {
        all,
        any: Vec::new(),
        sort,
    })
}

fn runtime_records_path(
    model_id: SymbolId,
    offset: usize,
    page_size: usize,
    criteria: &RuntimeRecordCriteria,
) -> std::result::Result<String, String> {
    let mut path = format!("/api/runtime/models/{model_id}/records?o={offset}&s={page_size}");
    if criteria.is_empty() {
        return Ok(path);
    }
    let criteria = serde_json::to_string(criteria)
        .map_err(|error| format!("序列化记录查询条件失败: {error}"))?;
    path.push_str("&criteria=");
    path.push_str(&urlencoding::encode(&criteria));
    Ok(path)
}

fn compiled_field(model: &CompiledModel, field_id: SymbolId) -> Option<(&str, &str, &ValueType)> {
    let slot = model.field_slots.get(&field_id)?;
    Some((
        model.field_names.get(slot)?.as_str(),
        model.field_titles.get(slot)?.as_str(),
        model.field_types.get(slot)?,
    ))
}

pub(crate) fn record_field<'a>(
    record: &'a RuntimeRecordView,
    model: &CompiledModel,
    field_id: SymbolId,
) -> Option<&'a Value> {
    let (name, _, _) = compiled_field(model, field_id)?;
    record.payload.get(name)
}

pub(crate) fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn form_text(event: &FormEvent, name: &str) -> String {
    match event.get_first(name) {
        Some(dioxus::html::FormValue::Text(value)) => value,
        _ => String::new(),
    }
}

fn render_runtime_error(message: &str) -> Element {
    rsx! {
        div { class: "aio-runtime-table-state is-error", "{message}" }
    }
}
