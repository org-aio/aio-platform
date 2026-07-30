use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use dioxus::prelude::*;
use icons::{
    ArrowDown, ArrowUp, ArrowUpDown, ChevronLeft, ChevronRight, Eye, Pencil, Plus, Search,
    Sparkles, Trash2, X,
};
use rudi::Context as RudiContext;
use serde_json::{Map, Value};

use crate::{
    CompiledModel, CompiledPage, CompiledPageRenderer, CompiledTable, CompiledTree,
    FormStateExtractionRequest, FormStateExtractionResponse, MenuActionAccess, MenuRowActions,
    ProgramImage, RuntimeRecordInput, RuntimeRecordPage, RuntimeRecordView, SymbolId, ValueType,
    browser_http::{delete_api, get_api, patch_api, post_api},
    design_system::{Button, ButtonSize, ButtonVariant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ConventionPageContext {
    pub route: String,
    pub page: CompiledPage,
}

pub trait ConventionPageProvider: Send + Sync + std::fmt::Debug {
    fn key(&self) -> &'static str;

    fn simple_name(&self) -> &'static str {
        let qualified_name = self.key();
        qualified_name
            .rsplit_once("::")
            .map_or(qualified_name, |(_, simple_name)| simple_name)
    }

    fn render(&self, context: ConventionPageContext) -> Element;
}

pub type DynConventionPageProvider = Arc<dyn ConventionPageProvider>;

#[derive(Clone, Debug, Default)]
pub struct ConventionPageIndex {
    providers: BTreeMap<String, DynConventionPageProvider>,
}

impl ConventionPageIndex {
    pub fn from_context(context: &mut RudiContext) -> Result<Self> {
        let provider_names = context
            .get_providers_by_type::<DynConventionPageProvider>()
            .into_iter()
            .map(|provider| provider.definition().key.name.to_string())
            .collect::<Vec<_>>();
        let mut providers = BTreeMap::new();
        for provider_name in provider_names {
            let provider = context
                .resolve_option_with_name::<DynConventionPageProvider>(provider_name.clone())
                .with_context(|| format!("无法解析约定页面 Provider: {provider_name}"))?;
            ensure!(
                provider.key() == provider_name,
                "约定页面的 Rudi name 与 Provider key 不一致: {provider_name} != {}",
                provider.key()
            );
            let simple_name = provider.simple_name().to_owned();
            if providers.insert(simple_name.clone(), provider).is_some() {
                bail!("约定页面模块名重复: {simple_name}");
            }
        }
        Ok(Self { providers })
    }

    pub fn render(&self, module_name: &str, context: ConventionPageContext) -> Option<Element> {
        self.providers
            .get(module_name)
            .map(|provider| provider.render(context))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RecordDialog {
    Create,
    Detail(RuntimeRecordView),
    Edit(RuntimeRecordView),
    Delete(RuntimeRecordView),
}

#[component]
pub fn BuiltInPage(
    api_base_url: String,
    image: ProgramImage,
    page: CompiledPage,
    row_actions: MenuRowActions,
) -> Element {
    let (table, tree) = match &page.renderer {
        CompiledPageRenderer::TreeTable { tree, table } => (table.clone(), Some(tree.clone())),
        CompiledPageRenderer::CrudTable { table } => (table.clone(), None),
        CompiledPageRenderer::ConventionFile { .. } => {
            return render_runtime_error("内置页面收到了约定文件渲染计划");
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
    let page_size = table.page_size as usize;
    let records_api = api_base_url.clone();
    let records_model = table.model_id;
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
        let relation_field_name = relation_field_name.clone();
        let _generation = generation();
        async move {
            let filter_query = match (relation_field_name.as_deref(), selected_tree.as_deref()) {
                (Some(field), Some(value)) => format!("&field={field}&value={value}"),
                _ => String::new(),
            };
            get_api::<RuntimeRecordPage>(
                &api_base_url,
                &format!(
                    "/api/runtime/models/{records_model}/records?o={current_offset}&s={page_size}{filter_query}"
                ),
            )
            .await
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
    let record_page = records.read().as_ref().cloned();
    let tree_page = tree_records.read().as_ref().cloned();
    let columns = table_columns(&model);
    let filter_fields = filter_fields(&model);
    let can_create = !matches!(row_actions.edit, MenuActionAccess::Hidden);
    let current_filters = filters();
    let mut rows = record_page
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|page| {
            page.d
                .iter()
                .filter(|record| record_matches(record, &model, &current_filters))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some((field_id, ascending)) = sort() {
        rows.sort_by(|left, right| {
            let ordering = record_field(left, &model, field_id)
                .map(value_to_text)
                .cmp(&record_field(right, &model, field_id).map(value_to_text));
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });
    }
    let total = record_page
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|page| page.t)
        .unwrap_or_default();
    let has_previous = offset() > 0;
    let has_next = offset().saturating_add(page_size) < total as usize;
    rsx! {
        section { class: "aio-runtime-table-page",
            header { class: "aio-runtime-table-page__header",
                div {
                    h2 { "{page.title}" }
                    p { "{model.title}" }
                }
                if can_create {
                    Button { onclick: move |_| dialog.set(Some(RecordDialog::Create)),
                        Plus { class: "size-4" }
                        "新增"
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
                        button {
                            class: if selected_tree().is_none() { "is-active" } else { "" },
                            onclick: move |_| selected_tree.set(None),
                            "全部"
                        }
                        match tree_page.as_ref() {
                            Some(Ok(Some(tree_page))) => rsx! {
                                for record in &tree_page.d {
                                    {tree_record_button(record, tree, &image, tree_page, selected_tree)}
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
                                    input { class: "aio-input", name: "{field_id}", placeholder: "{title}" }
                                }
                            }
                            Button { button_type: "submit",
                                Search { class: "size-4" }
                                "查询"
                            }
                            Button {
                                button_type: "reset",
                                variant: ButtonVariant::Outline,
                                onclick: move |_| {
                                    filters.set(BTreeMap::new());
                                    offset.set(0);
                                },
                                "重置"
                            }
                        }
                    }
                    div { class: "aio-runtime-table-wrap",
                        table {
                            thead { tr {
                                th { "序号" }
                                for field_id in &columns {
                                    if let Some((_, title, _)) = compiled_field(&model, *field_id) {
                                        th {
                                            if model.field_slots.get(field_id)
                                                .and_then(|slot| model.field_options.get(slot))
                                                .is_some_and(|options| options.sortable)
                                            {
                                                button {
                                                    class: "aio-runtime-sort",
                                                    title: "按 {title} 排序",
                                                    onclick: {
                                                        let field_id = *field_id;
                                                        move |_| sort.set(match sort() {
                                                            Some((current, ascending)) if current == field_id => {
                                                                Some((field_id, !ascending))
                                                            }
                                                            _ => Some((field_id, true)),
                                                        })
                                                    },
                                                    "{title}"
                                                    match sort() {
                                                        Some((current, true)) if current == *field_id => rsx! { ArrowUp { class: "size-3" } },
                                                        Some((current, false)) if current == *field_id => rsx! { ArrowDown { class: "size-3" } },
                                                        _ => rsx! { ArrowUpDown { class: "size-3" } },
                                                    }
                                                }
                                            } else {
                                                "{title}"
                                            }
                                        }
                                    }
                                }
                                th { "操作" }
                            } }
                            tbody {
                                for (index, record) in rows.iter().enumerate() {
                                    tr {
                                        td { "{offset() + index + 1}" }
                                        for field_id in &columns {
                                            td { "{record_field(record, &model, *field_id).map(value_to_text).unwrap_or_else(|| \"—\".to_owned())}" }
                                        }
                                        td { class: "aio-runtime-row-actions",
                                            if !matches!(row_actions.detail, MenuActionAccess::Hidden) {
                                                button { title: "详情", aria_label: "详情", onclick: {
                                                    let record = record.clone();
                                                    move |_| dialog.set(Some(RecordDialog::Detail(record.clone())))
                                                }, Eye { class: "size-4" } }
                                            }
                                            if !matches!(row_actions.edit, MenuActionAccess::Hidden) {
                                                button { title: "编辑", aria_label: "编辑", onclick: {
                                                    let record = record.clone();
                                                    move |_| dialog.set(Some(RecordDialog::Edit(record.clone())))
                                                }, Pencil { class: "size-4" } }
                                            }
                                            if !matches!(row_actions.delete, MenuActionAccess::Hidden) {
                                                button { class: "is-destructive", title: "删除", aria_label: "删除", onclick: {
                                                    let record = record.clone();
                                                    move |_| dialog.set(Some(RecordDialog::Delete(record.clone())))
                                                }, Trash2 { class: "size-4" } }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        match record_page {
                            Some(Err(error)) => rsx! { div { class: "aio-runtime-table-state is-error", "{error}" } },
                            None => rsx! { div { class: "aio-runtime-table-state", "正在加载" } },
                            Some(Ok(_)) if rows.is_empty() => rsx! { div { class: "aio-runtime-table-state", "暂无数据" } },
                            Some(Ok(_)) => rsx! {},
                        }
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
                    api_base_url,
                    model_id: table.model_id,
                    generation,
                    dialog,
                    notice,
                }
            }
        }
    }
}

fn tree_record_button(
    record: &RuntimeRecordView,
    tree: &CompiledTree,
    image: &ProgramImage,
    page: &RuntimeRecordPage,
    mut selected_tree: Signal<Option<String>>,
) -> Element {
    let Some(model) = image.models.get(&tree.model_id) else {
        return rsx! {};
    };
    let label = record_field(record, model, tree.label_field_id)
        .map(value_to_text)
        .unwrap_or_else(|| "未命名".to_owned());
    let depth = tree_depth(record, tree, model, &page.d);
    let indent = 0.75 + depth as f32;
    let record_id = record.id.clone();
    let active = selected_tree().as_deref() == Some(record.id.as_str());
    rsx! {
        button {
            class: if active { "is-active" } else { "" },
            style: "padding-left: {indent}rem",
            onclick: move |_| selected_tree.set(Some(record_id.clone())),
            "{label}"
        }
    }
}

fn tree_depth(
    record: &RuntimeRecordView,
    tree: &CompiledTree,
    model: &CompiledModel,
    records: &[RuntimeRecordView],
) -> usize {
    let Some(parent_field_id) = tree.parent_field_id else {
        return 0;
    };
    let mut current = record;
    let mut depth = 0;
    while depth < 8 {
        let Some(parent_id) = record_field(current, model, parent_field_id).map(value_to_text)
        else {
            break;
        };
        let Some(parent) = records.iter().find(|candidate| candidate.id == parent_id) else {
            break;
        };
        current = parent;
        depth += 1;
    }
    depth
}

#[component]
fn RuntimeRecordDialog(
    value: RecordDialog,
    model: CompiledModel,
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
    let submit_value = value.clone();
    let initial_form_state = initial_form_state(&model, record.as_ref());
    let mut form_state = use_signal(move || initial_form_state);
    let mut ai_prompt = use_signal(String::new);
    let ai_loading = use_signal(|| false);
    let can_ai_fill = !readonly
        && model
            .field_options
            .values()
            .any(|options| options.form_visible && options.form_editable && options.ai_extract);
    let submit_model = model.clone();
    rsx! {
        div { class: "aio-runtime-dialog-backdrop", onclick: move |_| dialog.set(None) }
        section { class: "aio-runtime-dialog", role: "dialog", aria_label: "{title}",
            header {
                h3 { "{title}" }
                Button { size: ButtonSize::IconSm, variant: ButtonVariant::Ghost, aria_label: "关闭", onclick: move |_| dialog.set(None),
                    X { class: "size-4" }
                }
            }
            if deleting {
                p { "删除后不可恢复，确认删除这条记录？" }
                footer {
                    Button { variant: ButtonVariant::Outline, onclick: move |_| dialog.set(None), "取消" }
                    Button { onclick: move |_| {
                        if let Some(record) = record.clone() {
                            delete_runtime_record(
                                api_base_url.clone(), model_id,
                                record.id, generation, dialog, notice,
                            );
                        }
                    }, "删除" }
                }
            } else {
                form { class: "aio-runtime-record-form", onsubmit: move |event| {
                    event.prevent_default();
                    if readonly {
                        dialog.set(None);
                        return;
                    }
                    let payload = record_payload_from_state(&submit_model, &form_state());
                    save_runtime_record(
                        api_base_url.clone(), model_id,
                        submit_value.clone(), payload, generation, dialog, notice,
                    );
                },
                    if can_ai_fill {
                        div { class: "aio-runtime-ai-fill",
                            textarea {
                                class: "aio-input",
                                aria_label: "AI 表单输入",
                                placeholder: "描述要填写的数据",
                                value: ai_prompt(),
                                oninput: move |event| ai_prompt.set(event.value()),
                            }
                            Button {
                                button_type: "button",
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
                        if let (Some(name), Some(title), Some(value_type)) = (
                            model.field_names.get(&slot),
                            model.field_titles.get(&slot),
                            model.field_types.get(&slot),
                        ) {
                            if model.field_options.get(&slot).is_some_and(|options| {
                                if readonly { options.detail_visible } else { options.form_visible }
                            }) {
                                label { "{title}" }
                                if matches!(value_type, ValueType::Boolean) {
                                    input {
                                        class: "aio-input",
                                        name: "{name}",
                                        r#type: "checkbox",
                                        disabled: readonly || model.field_options.get(&slot)
                                            .is_some_and(|options| !options.form_editable),
                                        checked: form_state().get(name)
                                            .is_some_and(|value| matches!(value.as_str(), "true" | "on" | "1")),
                                        onchange: {
                                            let name = name.clone();
                                            move |event| form_state.with_mut(|state| {
                                                state.insert(name.clone(), event.checked().to_string());
                                            })
                                        },
                                    }
                                } else {
                                    input {
                                        class: "aio-input",
                                        name: "{name}",
                                        r#type: field_input_type(value_type),
                                        required: model.required_fields.contains(&slot),
                                        readonly: readonly || model.field_options.get(&slot)
                                            .is_some_and(|options| !options.form_editable),
                                        placeholder: model.field_options.get(&slot)
                                            .and_then(|options| options.placeholder.as_deref())
                                            .unwrap_or_default(),
                                        value: form_state().get(name).cloned().unwrap_or_default(),
                                        oninput: {
                                            let name = name.clone();
                                            move |event| form_state.with_mut(|state| {
                                                state.insert(name.clone(), event.value());
                                            })
                                        },
                                    }
                                }
                                if let Some(help_text) = model.field_options.get(&slot)
                                    .and_then(|options| options.help_text.as_deref())
                                {
                                    small { "{help_text}" }
                                }
                            }
                        }
                    }
                    footer {
                        Button { button_type: "button", variant: ButtonVariant::Ghost, onclick: move |_| dialog.set(None), "取消" }
                        Button { button_type: "submit", if readonly { "关闭" } else { "保存" } }
                    }
                }
            }
        }
    }
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
    ai_loading.set(true);
    spawn(async move {
        let input = FormStateExtractionRequest {
            prompt,
            current_form_state: record_payload_from_state(&model, &form_state()),
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

fn record_payload_from_state(
    model: &CompiledModel,
    form_state: &BTreeMap<String, String>,
) -> Value {
    let payload = model
        .field_names
        .iter()
        .filter(|(slot, _)| {
            model
                .field_options
                .get(slot)
                .is_some_and(|options| options.form_visible)
        })
        .map(|(slot, name)| {
            let raw = form_state.get(name).cloned().unwrap_or_default();
            let value = model
                .field_types
                .get(slot)
                .map(|value_type| parse_field_value(value_type, &raw))
                .unwrap_or(Value::String(raw));
            (name.clone(), value)
        })
        .collect::<Map<_, _>>();
    Value::Object(payload)
}

fn parse_field_value(value_type: &ValueType, raw: &str) -> Value {
    match value_type {
        ValueType::Boolean => Value::Bool(matches!(raw, "true" | "on" | "1")),
        ValueType::Integer | ValueType::TimestampMs => {
            raw.parse::<i64>().map(Value::from).unwrap_or(Value::Null)
        }
        ValueType::Decimal => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueType::Null => Value::Null,
        ValueType::Optional { value } if raw.trim().is_empty() => Value::Null,
        ValueType::Optional { value } => parse_field_value(value, raw),
        ValueType::Any | ValueType::Object { .. } | ValueType::List { .. } => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
        }
        ValueType::Text | ValueType::File => Value::String(raw.to_owned()),
    }
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

fn compiled_field(model: &CompiledModel, field_id: SymbolId) -> Option<(&str, &str, &ValueType)> {
    let slot = model.field_slots.get(&field_id)?;
    Some((
        model.field_names.get(slot)?.as_str(),
        model.field_titles.get(slot)?.as_str(),
        model.field_types.get(slot)?,
    ))
}

fn record_field<'a>(
    record: &'a RuntimeRecordView,
    model: &CompiledModel,
    field_id: SymbolId,
) -> Option<&'a Value> {
    let (name, _, _) = compiled_field(model, field_id)?;
    record.payload.get(name)
}

fn record_matches(
    record: &RuntimeRecordView,
    model: &CompiledModel,
    filters: &BTreeMap<SymbolId, String>,
) -> bool {
    filters.iter().all(|(field_id, expected)| {
        record_field(record, model, *field_id).is_some_and(|value| {
            value_to_text(value)
                .to_lowercase()
                .contains(&expected.to_lowercase())
        })
    })
}

fn value_to_text(value: &Value) -> String {
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
