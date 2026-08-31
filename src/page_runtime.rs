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
    select::{Select, SelectItem},
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

include!("runtime_endpoint_form.rs");

include!("runtime_record_dialog.rs");

include!("runtime_table_support.rs");
