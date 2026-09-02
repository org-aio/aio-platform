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
                    aria_required: required,
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
                div {
                    id: input_id,
                    class: "aio-runtime-relation-select is-multiple grid gap-2 overflow-auto border",
                    aria_label: field_title,
                    for record in &rows {
                        {
                            let record_id = record.id.clone();
                            let record_label = relation_record_label(&target_model, record);
                            let record_selected = selected_ids.contains(&record.id);
                            let selected_ids = selected_ids.clone();
                            let field_name = field_name.clone();
                            rsx! {
                                label { class: "flex items-center gap-2",
                                    Checkbox {
                                        checked: Some(checkbox_state(record_selected)),
                                        name: field_name.clone(),
                                        value: record_id.clone(),
                                        disabled: select_disabled,
                                        aria_label: record_label.clone(),
                                        on_checked_change: move |checked| {
                                            let mut ids = selected_ids.clone();
                                            if checkbox_is_checked(checked) {
                                                if !ids.contains(&record_id) {
                                                    ids.push(record_id.clone());
                                                }
                                            } else {
                                                ids.retain(|id| id != &record_id);
                                            }
                                            let value = relation_form_state_value(relation.kind, ids);
                                            form_state.with_mut(|state| {
                                                state.insert(field_name.clone(), value);
                                            });
                                        },
                                    }
                                    span { "{record_label}" }
                                }
                            }
                        }
                    }
                }
            } else {
                Select {
                    id: input_id,
                    class: "aio-input aio-runtime-relation-select",
                    name: field_name.clone(),
                    aria_label: field_title,
                    aria_required: required,
                    disabled: select_disabled,
                    value: selected_ids.first().cloned().unwrap_or_default(),
                    options: std::iter::once(SelectItem::new(
                        "",
                        format!("请选择{}", target_model.title),
                    ))
                    .chain(rows.iter().map(|record| SelectItem::new(
                        record.id.clone(),
                        relation_record_label(&target_model, record),
                    )))
                    .collect(),
                    on_value_change: move |selected: String| {
                        let ids = (!selected.is_empty()).then_some(selected).into_iter().collect();
                        let value = relation_form_state_value(relation.kind, ids);
                        form_state.with_mut(|state| {
                            state.insert(field_name.clone(), value);
                        });
                    },
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

