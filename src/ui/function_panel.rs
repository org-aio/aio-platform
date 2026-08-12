use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum FunctionDesignerTab {
    #[default]
    Nodes,
    Ports,
    Edges,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FunctionPortEditorTarget {
    CreateInput,
    CreateOutput,
    EditInput(SymbolId),
    EditOutput(SymbolId),
}

impl FunctionPortEditorTarget {
    const fn is_input(self) -> bool {
        matches!(self, Self::CreateInput | Self::EditInput(_))
    }

    const fn port_id(self) -> Option<SymbolId> {
        match self {
            Self::EditInput(port_id) | Self::EditOutput(port_id) => Some(port_id),
            Self::CreateInput | Self::CreateOutput => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FunctionNodeEditorTarget {
    Create,
    Edit(SymbolId),
}

#[component]
pub(super) fn FunctionsPanel(
    draft: DraftSnapshot,
    api_base_url: String,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut selected_function: Signal<Option<SymbolId>>,
) -> Element {
    let mut function_search = use_signal(String::new);
    let mut creating_function = use_signal(|| false);
    let mut editing_function = use_signal(|| false);
    let mut deleting_function = use_signal(|| None::<DefinitionDeleteTarget>);
    let mut designer_tab = use_signal(FunctionDesignerTab::default);
    let mut port_editor = use_signal(|| None::<FunctionPortEditorTarget>);
    let deleting_port = use_signal(|| None::<DefinitionDeleteTarget>);
    let mut node_editor = use_signal(|| None::<FunctionNodeEditorTarget>);
    let deleting_node = use_signal(|| None::<DefinitionDeleteTarget>);
    let mut creating_edge = use_signal(|| false);
    let deleting_edge = use_signal(|| None::<GraphEdge>);
    let function_count = draft.definition.functions.len();
    let normalized_search = function_search().trim().to_lowercase();
    let visible_functions = draft
        .definition
        .functions
        .iter()
        .filter(|function| {
            definition_matches_search(&function.name, &function.title, &normalized_search)
        })
        .collect::<Vec<_>>();
    let current_function_id = selected_function()
        .filter(|selected_id| {
            visible_functions
                .iter()
                .any(|function| function.id == *selected_id)
        })
        .or_else(|| visible_functions.first().map(|function| function.id));
    let current_function = current_function_id.and_then(|function_id| {
        draft
            .definition
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .cloned()
    });
    let external_references = current_function_id.map_or(0, |function_id| {
        function_reference_count(&draft.definition, function_id)
    });
    let metadata_json = current_function
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose();
    let delete_title = if external_references > 0 {
        format!("该函数被 {external_references} 个函数节点调用，不能删除")
    } else {
        "删除函数".to_owned()
    };

    rsx! {
        section { class: "aio-function-workspace",
            nav { class: "aio-function-workspace__directory", aria_label: "函数目录",
                div { class: "aio-function-workspace__directory-heading",
                    div { class: "aio-function-workspace__directory-summary",
                        strong { "函数目录" }
                        div {
                            span { "{visible_functions.len()} / {function_count}" }
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "新建函数",
                                aria_label: "新建函数",
                                onclick: move |_| creating_function.set(true),
                                icons::Plus { class: "size-4" }
                            }
                        }
                    }
                    div { class: "aio-function-workspace__search",
                        Input {
                            class: "aio-input",
                            aria_label: "搜索函数",
                            placeholder: "搜索函数",
                            value: function_search(),
                            oninput: move |event: FormEvent| function_search.set(event.value()),
                        }
                        if !normalized_search.is_empty() {
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                title: "清除函数搜索",
                                aria_label: "清除函数搜索",
                                onclick: move |_| function_search.set(String::new()),
                                icons::X { class: "size-4" }
                            }
                        }
                    }
                }
                CollectionTree::<FunctionDefinition> {
                    class: "aio-function-workspace__directory-list",
                    aria_label: "函数目录",
                    data: CollectionTreeData::Collection(
                        visible_functions
                            .iter()
                            .map(|function| (*function).clone())
                            .collect()
                    ),
                    selected_key: current_function_id.map(|function_id| function_id.to_string()),
                    empty_text: "没有匹配的函数",
                    item_key: |function: FunctionDefinition| function.id.to_string(),
                    on_select: move |function: FunctionDefinition| {
                        selected_function.set(Some(function.id));
                    },
                    render_item: |item: CollectionTreeItemContext<FunctionDefinition>| {
                        let function = item.item;
                        rsx! {
                            span { class: "aio-function-workspace__function-content",
                                strong { "{function.title}" }
                                code { "{function.name}" }
                                span {
                                    "{function.inputs.len()} 入参 · {function.outputs.len()} 出参 · {function.graph.nodes.len()} 节点"
                                }
                            }
                        }
                    }
                }
            }
            main { class: "aio-function-workspace__editor",
                if let Some(function) = current_function.clone() {
                    header { class: "aio-function-workspace__editor-header",
                        div { class: "aio-function-workspace__identity",
                            h2 { "{function.title}" }
                            code { "{function.name}" }
                        }
                        div { class: "aio-function-workspace__metrics",
                            span { strong { "{function.inputs.len()}" } "入参" }
                            span { strong { "{function.outputs.len()}" } "出参" }
                            span { strong { "{function.graph.nodes.len()}" } "节点" }
                            span { strong { "{function.graph.edges.len()}" } "连线" }
                        }
                        div { class: "aio-function-workspace__actions",
                            Button {
                                r#type: "button",
                                size: ButtonSize::Sm,
                                variant: ButtonVariant::Outline,
                                onclick: move |_| editing_function.set(true),
                                icons::Pencil { class: "size-4" }
                                "编辑函数"
                            }
                            Button {
                                r#type: "button",
                                size: ButtonSize::IconSm,
                                variant: ButtonVariant::Ghost,
                                disabled: external_references > 0,
                                title: delete_title.clone(),
                                aria_label: delete_title.clone(),
                                onclick: {
                                    let function_title = function.title.clone();
                                    let function_id = function.id;
                                    move |_| deleting_function.set(Some(DefinitionDeleteTarget {
                                        id: function_id,
                                        kind: "函数",
                                        label: function_title.clone(),
                                    }))
                                },
                                icons::Trash2 { class: "size-4" }
                            }
                        }
                    }
                    section { class: "aio-function-workspace__summary",
                        dl {
                            div { dt { "权限" } dd { "{function.required_permissions.len()}" } }
                            div { dt { "外部调用" } dd { "{external_references}" } }
                            div { dt { "状态" } dd {
                                if function.state.is_known() { "完备" } else { "待补充" }
                            } }
                        }
                        div { class: "aio-function-workspace__permission-list",
                            if function.required_permissions.is_empty() {
                                span { "无权限限制" }
                            } else {
                                for permission_id in &function.required_permissions {
                                    Badge { variant: BadgeVariant::Outline,
                                        {draft.definition.permissions
                                            .iter()
                                            .find(|permission| permission.id == *permission_id)
                                            .map(|permission| permission.name.as_str())
                                            .unwrap_or("未知权限")}
                                    }
                                }
                            }
                        }
                    }
                    nav { class: "aio-function-workspace__tabs", aria_label: "函数设计视图",
                        Button {
                            r#type: "button",
                            class: if designer_tab() == FunctionDesignerTab::Nodes { "is-active" } else { "" },
                            variant: ButtonVariant::Ghost,
                            onclick: move |_| designer_tab.set(FunctionDesignerTab::Nodes),
                            "节点"
                            Badge { variant: BadgeVariant::Outline, "{function.graph.nodes.len()}" }
                        }
                        Button {
                            r#type: "button",
                            class: if designer_tab() == FunctionDesignerTab::Ports { "is-active" } else { "" },
                            variant: ButtonVariant::Ghost,
                            onclick: move |_| designer_tab.set(FunctionDesignerTab::Ports),
                            "端口"
                            Badge {
                                variant: BadgeVariant::Outline,
                                "{function.inputs.len() + function.outputs.len()}"
                            }
                        }
                        Button {
                            r#type: "button",
                            class: if designer_tab() == FunctionDesignerTab::Edges { "is-active" } else { "" },
                            variant: ButtonVariant::Ghost,
                            onclick: move |_| designer_tab.set(FunctionDesignerTab::Edges),
                            "连线"
                            Badge { variant: BadgeVariant::Outline, "{function.graph.edges.len()}" }
                        }
                    }
                    match designer_tab() {
                        FunctionDesignerTab::Nodes => rsx! {
                            FunctionGraphCanvas {
                                function,
                                models: draft.definition.models.clone(),
                                routes: draft.definition.routes.clone(),
                                api_base_url: api_base_url.clone(),
                                program_id: draft.program_id.clone(),
                                version: draft.version,
                                generation,
                                status,
                                editor: node_editor,
                                deleting: deleting_node,
                            }
                        },
                        FunctionDesignerTab::Ports => rsx! {
                            FunctionPortsPanel {
                                function,
                                editor: port_editor,
                                deleting: deleting_port,
                            }
                        },
                        FunctionDesignerTab::Edges => rsx! {
                            FunctionEdgesPanel {
                                function,
                                creating: creating_edge,
                                deleting: deleting_edge,
                            }
                        },
                    }
                } else {
                    div { class: "aio-function-workspace__empty", "暂无函数" }
                }
            }
            aside { class: "aio-function-workspace__metadata",
                header {
                    div {
                        strong { "函数 JSON" }
                        if let Some(function) = current_function.as_ref() {
                            code { "{function.name}" }
                        }
                    }
                    if let Ok(Some(json)) = &metadata_json {
                        Button {
                            size: ButtonSize::Sm,
                            variant: ButtonVariant::Outline,
                            title: "复制函数 JSON",
                            onclick: {
                                let json = json.clone();
                                move |_| copy_json_to_clipboard(json.clone(), status)
                            },
                            icons::Copy { class: "size-4" }
                            "复制"
                        }
                    }
                }
                match &metadata_json {
                    Ok(Some(json)) => rsx! { pre { "{json}" } },
                    Err(error) => rsx! {
                        div { class: "aio-function-workspace__empty",
                            "函数元数据序列化失败: {error}"
                        }
                    },
                    Ok(None) => rsx! {
                        div { class: "aio-function-workspace__empty", "暂无函数元数据" }
                    },
                }
            }
            if creating_function() {
                FunctionDefinitionDialog {
                    function: None,
                    functions: draft.definition.functions.clone(),
                    permissions: draft.definition.permissions.clone(),
                    root_id: draft.definition.id,
                    function_count,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| creating_function.set(false),
                    on_saved: move |function_id| {
                        function_search.set(String::new());
                        selected_function.set(Some(function_id));
                        creating_function.set(false);
                    },
                }
            }
            if editing_function()
                && let Some(function) = current_function.clone()
            {
                FunctionDefinitionDialog {
                    function: Some(function),
                    functions: draft.definition.functions.clone(),
                    permissions: draft.definition.permissions.clone(),
                    root_id: draft.definition.id,
                    function_count,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| editing_function.set(false),
                    on_saved: move |function_id| {
                        selected_function.set(Some(function_id));
                        editing_function.set(false);
                    },
                }
            }
            if let Some(target) = port_editor()
                && let Some(function) = current_function.clone()
            {
                FunctionPortDialog {
                    key: "function-port:{function.id}:{target:?}",
                    port: target.port_id().and_then(|port_id| {
                        function
                            .inputs
                            .iter()
                            .chain(&function.outputs)
                            .find(|port| port.id == port_id)
                            .cloned()
                    }),
                    input: target.is_input(),
                    function,
                    models: draft.definition.models.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| port_editor.set(None),
                    on_saved: move |_| port_editor.set(None),
                }
            }
            if let Some(target) = deleting_port() {
                DefinitionDeleteDialog {
                    target,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    deleting: deleting_port,
                    on_deleted: move |_| {},
                }
            }
            if let Some(target) = node_editor()
                && let Some(function) = current_function.clone()
            {
                FunctionNodeDialog {
                    key: "function-node:{function.id}:{target:?}",
                    node: match target {
                        FunctionNodeEditorTarget::Create => None,
                        FunctionNodeEditorTarget::Edit(node_id) => function
                            .graph
                            .nodes
                            .iter()
                            .find(|node| node.id == node_id)
                            .cloned(),
                    },
                    function,
                    models: draft.definition.models.clone(),
                    routes: draft.definition.routes.clone(),
                    functions: draft.definition.functions.clone(),
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| node_editor.set(None),
                    on_saved: move |_| node_editor.set(None),
                }
            }
            if let Some(target) = deleting_node() {
                DefinitionDeleteDialog {
                    target,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    deleting: deleting_node,
                    on_deleted: move |_| {},
                }
            }
            if creating_edge()
                && let Some(function) = current_function.clone()
            {
                FunctionEdgeDialog {
                    function,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    on_close: move |_| creating_edge.set(false),
                    on_saved: move |_| creating_edge.set(false),
                }
            }
            if let Some(edge) = deleting_edge()
                && let Some(function) = current_function.clone()
            {
                FunctionEdgeDeleteDialog {
                    function_id: function.id,
                    edge,
                    api_base_url: api_base_url.clone(),
                    program_id: draft.program_id.clone(),
                    version: draft.version,
                    generation,
                    status,
                    deleting: deleting_edge,
                }
            }
            if let Some(target) = deleting_function() {
                DefinitionDeleteDialog {
                    target,
                    api_base_url,
                    program_id: draft.program_id,
                    version: draft.version,
                    generation,
                    status,
                    deleting: deleting_function,
                    on_deleted: move |_| selected_function.set(None),
                }
            }
        }
    }
}
