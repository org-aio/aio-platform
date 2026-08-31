use super::*;

#[component]
pub(super) fn FunctionEdgeDialog(
    function: FunctionDefinition,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let first_node_id = function
        .graph
        .nodes
        .first()
        .map(|node| node.id.to_string())
        .unwrap_or_default();
    let second_node_id = function
        .graph
        .nodes
        .get(1)
        .map(|node| node.id.to_string())
        .unwrap_or_default();
    let mut from_node_id = use_signal(move || first_node_id);
    let mut to_node_id = use_signal(move || second_node_id);
    rsx! {
        Dialog {
            class: "aio-definition-dialog aio-function-edge-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    on_close.call(());
                }
            },
            header { class: "aio-definition-dialog__header",
                div {
                    DialogTitle { "新建节点连线" }
                    DialogDescription { "{function.title} · {function.name}" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::IconSm,
                    variant: ButtonVariant::Ghost,
                    title: "关闭连线编辑",
                    aria_label: "关闭连线编辑",
                    onclick: move |_| on_close.call(()),
                    icons::X { class: "size-4" }
                }
            }
            form { class: "aio-definition-dialog__form", onsubmit: move |event| {
                event.prevent_default();
                let Ok(from_node) = SymbolId::parse(&from_node_id()) else {
                    status.set(Some("请选择起点节点".to_owned()));
                    return;
                };
                let Ok(to_node) = SymbolId::parse(&to_node_id()) else {
                    status.set(Some("请选择终点节点".to_owned()));
                    return;
                };
                if from_node == to_node {
                    status.set(Some("连线起点和终点不能相同".to_owned()));
                    return;
                }
                let Some(from_kind) = function
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == from_node)
                    .map(|node| &node.kind)
                else {
                    status.set(Some("连线起点不存在".to_owned()));
                    return;
                };
                let Some(to_kind) = function
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == to_node)
                    .map(|node| &node.kind)
                else {
                    status.set(Some("连线终点不存在".to_owned()));
                    return;
                };
                if !crate::function_nodes_can_connect(from_kind, to_kind) {
                    status.set(Some("所选节点不允许建立这条连线".to_owned()));
                    return;
                }
                if function.graph.edges.iter().any(|edge| {
                    edge.from_node == from_node
                        && edge.to_node == to_node
                        && edge.from_port == "out"
                        && edge.to_port == "in"
                }) {
                    status.set(Some("相同节点与端口的连线已存在".to_owned()));
                    return;
                }
                let edge = GraphEdge {
                    id: SymbolId::new(),
                    from_node,
                    from_port: "out".to_owned(),
                    to_node,
                    to_port: "in".to_owned(),
                };
                submit_patches(
                    api_base_url.clone(),
                    program_id.clone(),
                    version,
                    vec![GraphPatch::Connect {
                        function_id: function.id,
                        edge,
                    }],
                    generation,
                    status,
                );
                on_saved.call(());
            },
                div { class: "aio-definition-dialog__grid",
                    label { "起点节点"
                        Select {
                            class: "aio-input",
                            aria_label: "连线起点节点",
                            value: from_node_id(),
                            options: function.graph.nodes.iter().map(|node| SelectItem::new(
                                node.id.to_string(),
                                format!("{} · {}", node.name, function_node_kind_title(&node.kind)),
                            )).collect(),
                            on_value_change: move |value: String| from_node_id.set(value),
                        }
                    }
                    label { "终点节点"
                        Select {
                            class: "aio-input",
                            aria_label: "连线终点节点",
                            value: to_node_id(),
                            options: function.graph.nodes.iter().map(|node| SelectItem::new(
                                node.id.to_string(),
                                format!("{} · {}", node.name, function_node_kind_title(&node.kind)),
                            )).collect(),
                            on_value_change: move |value: String| to_node_id.set(value),
                        }
                    }
                }
                footer { class: "aio-definition-dialog__actions",
                    Button {
                        r#type: "button",
                        variant: ButtonVariant::Ghost,
                        onclick: move |_| on_close.call(()),
                        "取消"
                    }
                    Button {
                        r#type: "submit",
                        icons::Save { class: "size-4" }
                        "创建连线"
                    }
                }
            }
        }
    }
}

#[component]
pub(super) fn FunctionEdgeDeleteDialog(
    function_id: SymbolId,
    edge: GraphEdge,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    status: Signal<Option<String>>,
    mut deleting: Signal<Option<GraphEdge>>,
) -> Element {
    let edge_id = edge.id;
    rsx! {
        Dialog {
            class: "aio-definition-confirm-dialog",
            open: true,
            on_open_change: move |open: bool| {
                if !open {
                    deleting.set(None);
                }
            },
            DialogTitle { "删除节点连线" }
            DialogDescription {
                "确认删除 {edge.from_port} → {edge.to_port} 连线？此操作不可恢复。"
            }
            footer { class: "aio-definition-dialog__actions",
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Ghost,
                    onclick: move |_| deleting.set(None),
                    "取消"
                }
                Button {
                    r#type: "button",
                    variant: ButtonVariant::Destructive,
                    onclick: move |_| {
                        submit_patches(
                            api_base_url.clone(),
                            program_id.clone(),
                            version,
                            vec![GraphPatch::Disconnect {
                                function_id,
                                edge_id,
                            }],
                            generation,
                            status,
                        );
                        deleting.set(None);
                    },
                    icons::Trash2 { class: "size-4" }
                    "删除"
                }
            }
        }
    }
}
