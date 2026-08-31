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
