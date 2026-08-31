use super::*;

#[component]
pub(super) fn ModelFieldInlineCellEditor(
    edit: DataTableEditContext<ModelFieldRow>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
) -> Element {
    let ModelFieldRow::Field(field) = edit.cell.row.clone() else {
        return rsx! { "—" };
    };
    let column = edit.cell.column.key.clone();
    let close = edit.close;

    if column == "type" {
        let current_type = editable_value_type_key(&field.value_type).to_owned();
        return rsx! {
            Select {
                aria_label: "编辑字段类型 {field.title}",
                value: current_type,
                options: editable_value_type_options(&field.value_type),
                on_value_change: move |value: String| {
                    let next_type = editable_value_type_from_key(&value, &field.value_type);
                    if next_type != field.value_type {
                        submit_patches(
                            api_base_url.clone(),
                            program_id.clone(),
                            version,
                            vec![GraphPatch::SetProperty {
                                target_id: field.id,
                                property: crate::EditableProperty::FieldValueType,
                                value: serde_json::json!(next_type),
                            }],
                            generation,
                            status,
                        );
                    }
                    close.call(());
                },
            }
        };
    }

    let initial_title = field.title.clone();
    let field_name_for_label = field.name.clone();
    let mut title = use_signal(move || initial_title);
    let mut submitted = use_signal(|| false);
    let submit = use_callback(move |_: ()| {
        if submitted() {
            return;
        }
        let next_title = title().trim().to_owned();
        if next_title.is_empty() {
            status.set(Some("字段标题不能为空".to_owned()));
            return;
        }
        if next_title == field.title {
            close.call(());
            return;
        }
        submitted.set(true);
        submit_patches(
            api_base_url.clone(),
            program_id.clone(),
            version,
            vec![GraphPatch::Rename {
                target_id: field.id,
                name: field.name.clone(),
                title: Some(next_title),
            }],
            generation,
            status,
        );
        close.call(());
    });

    rsx! {
        Input {
            class: "aio-input",
            value: title(),
            aria_label: "编辑字段标题 {field_name_for_label}",
            onmounted: move |event: MountedEvent| async move {
                let _ = event.data().set_focus(true).await;
            },
            oninput: move |event: FormEvent| title.set(event.value()),
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

#[allow(clippy::too_many_arguments)]
pub(super) fn model_field_toggle_cell(
    field: FieldDefinition,
    column: &str,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
) -> Element {
    let checked = field_toggle_value(&field, column);
    let aria_label = format!("切换字段 {} 的{}", field.title, field_toggle_label(column));
    let column = column.to_owned();
    rsx! {
        Checkbox {
            checked: Some(checkbox_state(checked)),
            aria_label,
            on_checked_change: move |state| {
                let checked = checkbox_is_checked(state);
                let patch = GraphPatch::SetProperty {
                    target_id: field.id,
                    property: field_toggle_property(&column),
                    value: serde_json::json!(checked),
                };
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    vec![patch],
                    generation,
                    status,
                );
            },
        }
    }
}

pub(super) fn field_toggle_value(field: &FieldDefinition, column: &str) -> bool {
    match column {
        "required" => field.required,
        "list_visible" => field.options.list_visible,
        "detail_visible" => field.options.detail_visible,
        "form_visible" => field.options.form_visible,
        "form_editable" => field.options.form_editable,
        "filterable" => field.options.filterable,
        "sortable" => field.options.sortable,
        "unique" => field.options.unique,
        _ => false,
    }
}

fn field_toggle_property(column: &str) -> crate::EditableProperty {
    match column {
        "required" => crate::EditableProperty::FieldRequired,
        "list_visible" => crate::EditableProperty::FieldListVisible,
        "detail_visible" => crate::EditableProperty::FieldDetailVisible,
        "form_visible" => crate::EditableProperty::FieldFormVisible,
        "form_editable" => crate::EditableProperty::FieldFormEditable,
        "filterable" => crate::EditableProperty::FieldFilterable,
        "sortable" => crate::EditableProperty::FieldSortable,
        "unique" => crate::EditableProperty::FieldUnique,
        _ => unreachable!("未知字段表格开关列: {column}"),
    }
}

fn field_toggle_label(column: &str) -> &'static str {
    match column {
        "required" => "必填",
        "list_visible" => "列表显示",
        "detail_visible" => "详情显示",
        "form_visible" => "表单显示",
        "form_editable" => "表单可编辑",
        "filterable" => "允许查询",
        "sortable" => "允许排序",
        "unique" => "值唯一",
        _ => "字段能力",
    }
}
