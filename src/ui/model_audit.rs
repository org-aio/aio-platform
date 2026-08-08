use super::*;

#[component]
pub(super) fn ModelAuditDialog(
    model: ModelDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut editor: Signal<Option<ModelEditorTarget>>,
) -> Element {
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-audit-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    editor.set(None);
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "配置审计字段" }
                    DialogDescription { "{model.title} · 自动创建并绑定审计语义字段" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭审计字段配置",
                    aria_label: "关闭审计字段配置",
                    onclick: move |_| editor.set(None),
                    icons::X { class: "size-4" }
                }
            }
            div { class: "aio-definition-dialog__body",
                ModelAuditEditor {
                    model_id: model.id,
                    fields: model.fields,
                    audit: model.audit,
                    api_base_url,
                    program_id,
                    version,
                    generation,
                    status,
                    on_saved: move |_| editor.set(None),
                }
            }
        }
    }
}

#[component]
pub(super) fn ModelAuditEditor(
    model_id: SymbolId,
    fields: Vec<FieldDefinition>,
    audit: crate::ModelAuditDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_saved: EventHandler<()>,
) -> Element {
    let initial_kinds = audit
        .fields
        .iter()
        .map(|field| field.kind)
        .collect::<BTreeSet<_>>();
    let bindings = audit
        .fields
        .iter()
        .map(|field| (field.kind, field.field_id))
        .collect::<BTreeMap<_, _>>();
    let mut selected = use_signal(move || initial_kinds);
    rsx! {
        form {
            class: "aio-model-audit-editor",
            onsubmit: move |event| {
                event.prevent_default();
                let selected_kinds = selected();
                let mut audit_fields = Vec::with_capacity(selected_kinds.len());
                let mut patches = Vec::new();
                let mut next_field_index = fields.len();
                for kind in crate::AuditFieldKind::all() {
                    if !selected_kinds.contains(&kind) {
                        continue;
                    }
                    let field_id = if let Some(field_id) = bindings.get(&kind) {
                        *field_id
                    } else if let Some(field) = fields
                        .iter()
                        .find(|field| field.name == kind.default_name())
                    {
                        if field.value_type != kind.default_value_type() {
                            status.set(Some(format!(
                                "审计字段 {} 必须使用 {} 类型",
                                kind.default_name(),
                                value_type_label(&kind.default_value_type())
                            )));
                            return;
                        }
                        field.id
                    } else {
                        let field_id = SymbolId::new();
                        let field = audit_field_definition(kind, field_id);
                        patches.push(GraphPatch::Insert {
                            parent_id: model_id,
                            collection: ChildCollection::Fields,
                            index: next_field_index,
                            entity: GraphEntity::Field(field),
                        });
                        next_field_index = next_field_index.saturating_add(1);
                        field_id
                    };
                    audit_fields.push(crate::ModelAuditField { kind, field_id });
                }
                patches.push(GraphPatch::SetProperty {
                    target_id: model_id,
                    property: crate::EditableProperty::ModelAudit,
                    value: serde_json::json!(crate::ModelAuditDefinition {
                        fields: audit_fields,
                    }),
                });
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    patches,
                    generation,
                    status,
                );
                on_saved.call(());
            },
            div { class: "aio-model-audit-editor__roles",
                for kind in crate::AuditFieldKind::all() {
                    label {
                        Checkbox {
                            checked: Some(checkbox_state(selected().contains(&kind))),
                            aria_label: "启用审计字段 {kind.label()}",
                            on_checked_change: move |checked| selected.with_mut(|kinds| {
                                if checkbox_is_checked(checked) {
                                    kinds.insert(kind);
                                } else {
                                    kinds.remove(&kind);
                                }
                            }),
                        }
                        span { "{kind.label()}" }
                        code { "{kind.default_name()}" }
                    }
                }
            }
            footer {
                Button {
                    r#type: "submit",
                    size: ButtonSize::Sm,
                    variant: ButtonVariant::Outline,
                    title: "保存审计字段",
                    aria_label: "保存审计字段",
                    icons::Save { class: "size-4" }
                    "保存审计字段"
                }
            }
        }
    }
}

pub(super) fn audit_field_definition(kind: crate::AuditFieldKind, id: SymbolId) -> FieldDefinition {
    let mut options = crate::FieldOptions::default();
    options.form_visible = false;
    options.form_editable = false;
    options.excel_import = false;
    options.ai_extract = false;
    options.filterable = matches!(
        kind,
        crate::AuditFieldKind::TenantId | crate::AuditFieldKind::Deleted
    );
    options.sortable = matches!(
        kind,
        crate::AuditFieldKind::CreatedAt
            | crate::AuditFieldKind::UpdatedAt
            | crate::AuditFieldKind::DeletedAt
            | crate::AuditFieldKind::Version
    );
    FieldDefinition {
        id,
        name: kind.default_name().to_owned(),
        title: kind.default_title().to_owned(),
        value_type: kind.default_value_type(),
        state: DefinitionState::Known,
        required: false,
        options,
        relation: None,
    }
}
