use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FunctionNodeDragState {
    pointer_id: i32,
    node_id: SymbolId,
    start_client_x: f64,
    start_client_y: f64,
    original: FunctionNodeEditor,
    current: FunctionNodeEditor,
}

pub(super) fn dragged_function_node_editor(
    state: FunctionNodeDragState,
    client_x: f64,
    client_y: f64,
) -> FunctionNodeEditor {
    let delta_x = (client_x - state.start_client_x).round() as i32;
    let delta_y = (client_y - state.start_client_y).round() as i32;
    FunctionNodeEditor {
        x: state.original.x.saturating_add(delta_x).clamp(0, 10_000),
        y: state.original.y.saturating_add(delta_y).clamp(0, 10_000),
    }
}

#[component]
pub(super) fn FunctionGraphCanvas(
    function: FunctionDefinition,
    models: Vec<ModelDefinition>,
    routes: Vec<RouteDefinition>,
    api_base_url: String,
    program_id: String,
    version: i64,
    generation: Signal<u64>,
    mut status: Signal<Option<String>>,
    mut editor: Signal<Option<FunctionNodeEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let mut drag_state = use_signal(|| None::<FunctionNodeDragState>);
    let mut connection_source = use_signal(|| None::<SymbolId>);
    let active_drag = drag_state();
    let effective_editors = function
        .graph
        .nodes
        .iter()
        .map(|node| {
            let editor = active_drag
                .filter(|drag| drag.node_id == node.id)
                .map_or(node.editor, |drag| drag.current);
            (node.id, editor)
        })
        .collect::<BTreeMap<_, _>>();
    let canvas_width = effective_editors
        .values()
        .map(|editor| editor.x.max(0) + 260)
        .max()
        .unwrap_or(0)
        .max(720);
    let canvas_height = effective_editors
        .values()
        .map(|editor| editor.y.max(0) + 180)
        .max()
        .unwrap_or(0)
        .max(400);
    let node_positions = effective_editors
        .iter()
        .map(|(node_id, editor)| (*node_id, (editor.x.max(0) + 24, editor.y.max(0) + 24)))
        .collect::<BTreeMap<_, _>>();
    let edge_lines = function
        .graph
        .edges
        .iter()
        .filter_map(|edge| {
            let (from_x, from_y) = node_positions.get(&edge.from_node)?;
            let (to_x, to_y) = node_positions.get(&edge.to_node)?;
            Some((edge.id, from_x + 208, from_y + 36, *to_x, to_y + 36))
        })
        .collect::<Vec<_>>();
    let connection_source_kind = connection_source().and_then(|source_id| {
        function
            .graph
            .nodes
            .iter()
            .find(|node| node.id == source_id)
            .map(|node| node.kind.clone())
    });
    let node_cards = function
        .graph
        .nodes
        .iter()
        .map(|node| {
            let editor = effective_editors
                .get(&node.id)
                .copied()
                .unwrap_or(node.editor);
            (
                node.clone(),
                editor.x.max(0) + 24,
                editor.y.max(0) + 24,
                function_node_detail(&node.kind, &models, &routes),
                function_node_reference_count(&function, node.id),
                connection_source_kind
                    .as_ref()
                    .is_some_and(|source| crate::function_nodes_can_connect(source, &node.kind)),
                !matches!(&node.kind, FunctionNodeKind::Fail { .. }),
            )
        })
        .collect::<Vec<_>>();
    let function_id = function.id;
    let direct_edges = function
        .graph
        .edges
        .iter()
        .filter(|edge| edge.from_port == "out" && edge.to_port == "in")
        .map(|edge| (edge.from_node, edge.to_node))
        .collect::<BTreeSet<_>>();
    let connection_source_name = connection_source().and_then(|source_id| {
        function
            .graph
            .nodes
            .iter()
            .find(|node| node.id == source_id)
            .map(|node| node.name.clone())
    });

    rsx! {
        section { class: "aio-function-graph",
            header {
                div {
                    h3 { "函数图" }
                    p { "{function.graph.nodes.len()} 个节点 · {function.graph.edges.len()} 条连线" }
                }
                div { class: "aio-function-graph__actions",
                    if let Some(source_name) = connection_source_name {
                        Badge { variant: BadgeVariant::Outline, "连接自 {source_name}" }
                        Button {
                            r#type: "button",
                            size: ButtonSize::IconSm,
                            variant: ButtonVariant::Ghost,
                            title: "取消节点连线",
                            aria_label: "取消节点连线",
                            onclick: move |_| connection_source.set(None),
                            icons::X { class: "size-4" }
                        }
                    }
                    Badge { variant: BadgeVariant::Outline, "结构化 IR" }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| editor.set(Some(FunctionNodeEditorTarget::Create)),
                        icons::Plus { class: "size-4" }
                        "新建节点"
                    }
                }
            }
            div { class: "aio-function-graph__viewport",
                if function.graph.nodes.is_empty() {
                    div { class: "aio-function-graph__empty", "暂无函数节点" }
                } else {
                    div {
                        class: "aio-function-graph__canvas",
                        style: "width: {canvas_width}px; height: {canvas_height}px;",
                        onpointermove: move |event: PointerEvent| {
                            let Some(mut drag) = drag_state() else {
                                return;
                            };
                            if drag.pointer_id != event.data().pointer_id() {
                                return;
                            }
                            event.prevent_default();
                            let point = event.data().client_coordinates();
                            drag.current = dragged_function_node_editor(drag, point.x, point.y);
                            drag_state.set(Some(drag));
                        },
                        onpointerup: {
                            let api_base_url = api_base_url.clone();
                            let program_id = program_id.clone();
                            move |event: PointerEvent| {
                                let Some(drag) = drag_state() else {
                                    return;
                                };
                                if drag.pointer_id != event.data().pointer_id() {
                                    return;
                                }
                                drag_state.set(None);
                                if drag.current == drag.original {
                                    return;
                                }
                                let patch = GraphPatch::SetProperty {
                                    target_id: drag.node_id,
                                    property: crate::EditableProperty::FunctionNodePosition,
                                    value: serde_json::json!(drag.current),
                                };
                                submit_patches(
                                    api_base_url.clone(),
                                    program_id.clone(),
                                    version,
                                    vec![patch],
                                    generation,
                                    status,
                                );
                            }
                        },
                        onpointercancel: move |_| drag_state.set(None),
                        onpointerleave: move |_| drag_state.set(None),
                        svg {
                            class: "aio-function-graph__edges",
                            view_box: "0 0 {canvas_width} {canvas_height}",
                            preserve_aspect_ratio: "none",
                            for (edge_id, from_x, from_y, to_x, to_y) in edge_lines {
                                line {
                                    key: "{edge_id}",
                                    x1: "{from_x}",
                                    y1: "{from_y}",
                                    x2: "{to_x}",
                                    y2: "{to_y}",
                                }
                            }
                        }
                        for (node, node_left, node_top, node_detail, reference_count, can_receive_connection, can_emit_connection) in node_cards {
                            article {
                                key: "{node.id}",
                                class: if active_drag.is_some_and(|drag| drag.node_id == node.id) {
                                    "aio-function-node is-dragging"
                                } else if connection_source() == Some(node.id) {
                                    "aio-function-node is-connection-source"
                                } else {
                                    "aio-function-node"
                                },
                                style: "left: {node_left}px; top: {node_top}px;",
                                aria_label: "函数节点 {node.name}",
                                Button {
                                    class: "aio-function-node__port aio-function-node__port--input",
                                    r#type: "button",
                                    size: ButtonSize::IconXs,
                                    variant: ButtonVariant::Outline,
                                    disabled: !can_receive_connection,
                                    title: "连接到 {node.name}",
                                    aria_label: "连接到节点 {node.name}",
                                    onclick: {
                                        let target_node_id = node.id;
                                        let api_base_url = api_base_url.clone();
                                        let program_id = program_id.clone();
                                        let direct_edges = direct_edges.clone();
                                        move |event: MouseEvent| {
                                            event.stop_propagation();
                                            let Some(source_node_id) = connection_source() else {
                                                return;
                                            };
                                            if source_node_id == target_node_id {
                                                return;
                                            }
                                            if !can_receive_connection {
                                                status.set(Some("这两个节点不能建立输入连线".to_owned()));
                                                return;
                                            }
                                            if direct_edges.contains(&(source_node_id, target_node_id)) {
                                                status.set(Some("两个节点之间已存在默认连线".to_owned()));
                                                return;
                                            }
                                            let edge = GraphEdge {
                                                id: SymbolId::new(),
                                                from_node: source_node_id,
                                                from_port: "out".to_owned(),
                                                to_node: target_node_id,
                                                to_port: "in".to_owned(),
                                            };
                                            connection_source.set(None);
                                            submit_patches(
                                                api_base_url.clone(),
                                                program_id.clone(),
                                                version,
                                                vec![GraphPatch::Connect {
                                                    function_id,
                                                    edge,
                                                }],
                                                generation,
                                                status,
                                            );
                                        }
                                    },
                                    icons::Link { class: "size-3" }
                                }
                                Button {
                                    class: if connection_source() == Some(node.id) {
                                        "aio-function-node__port aio-function-node__port--output is-active"
                                    } else {
                                        "aio-function-node__port aio-function-node__port--output"
                                    },
                                    r#type: "button",
                                    size: ButtonSize::IconXs,
                                    variant: ButtonVariant::Outline,
                                    disabled: !can_emit_connection,
                                    title: if connection_source() == Some(node.id) {
                                        format!("取消从 {} 连线", node.name)
                                    } else {
                                        format!("从 {} 开始连线", node.name)
                                    },
                                    aria_label: if connection_source() == Some(node.id) {
                                        format!("取消从节点 {} 连线", node.name)
                                    } else {
                                        format!("从节点 {} 开始连线", node.name)
                                    },
                                    onclick: {
                                        let source_node_id = node.id;
                                        move |event: MouseEvent| {
                                            event.stop_propagation();
                                            if connection_source() == Some(source_node_id) {
                                                connection_source.set(None);
                                            } else {
                                                connection_source.set(Some(source_node_id));
                                            }
                                        }
                                    },
                                    icons::Link { class: "size-3" }
                                }
                                header {
                                    title: "拖动移动节点",
                                    onpointerdown: {
                                        let node_id = node.id;
                                        let original = node.editor;
                                        move |event: PointerEvent| {
                                            event.prevent_default();
                                            event.stop_propagation();
                                            let point = event.data().client_coordinates();
                                            drag_state.set(Some(FunctionNodeDragState {
                                                pointer_id: event.data().pointer_id(),
                                                node_id,
                                                start_client_x: point.x,
                                                start_client_y: point.y,
                                                original,
                                                current: original,
                                            }));
                                        }
                                    },
                                    strong { "{node.name}" }
                                    Badge { variant: BadgeVariant::Outline,
                                        {function_node_kind_title(&node.kind)}
                                    }
                                }
                                p { "{node_detail}" }
                                footer {
                                    Button {
                                        r#type: "button",
                                        size: ButtonSize::IconXs,
                                        variant: ButtonVariant::Ghost,
                                        title: "编辑节点 {node.name}",
                                        aria_label: "编辑节点 {node.name}",
                                        onclick: {
                                            let node_id = node.id;
                                            move |_| editor.set(Some(FunctionNodeEditorTarget::Edit(node_id)))
                                        },
                                        icons::Pencil { class: "size-3" }
                                    }
                                    Button {
                                        r#type: "button",
                                        size: ButtonSize::IconXs,
                                        variant: ButtonVariant::Ghost,
                                        disabled: reference_count > 0,
                                        title: if reference_count > 0 {
                                            format!("该节点被 {reference_count} 个节点引用，不能删除")
                                        } else {
                                            format!("删除节点 {}", node.name)
                                        },
                                        aria_label: if reference_count > 0 {
                                            format!("该节点被 {reference_count} 个节点引用，不能删除")
                                        } else {
                                            format!("删除节点 {}", node.name)
                                        },
                                        onclick: {
                                            let node_id = node.id;
                                            let node_name = node.name.clone();
                                            move |_| deleting.set(Some(DefinitionDeleteTarget {
                                                id: node_id,
                                                kind: "函数节点",
                                                label: node_name.clone(),
                                            }))
                                        },
                                        icons::Trash2 { class: "size-3" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FunctionPortDirection {
    Input,
    Output,
}

impl FunctionPortDirection {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Input => "输入",
            Self::Output => "输出",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FunctionPortRow {
    pub(super) direction: FunctionPortDirection,
    pub(super) port: PortDefinition,
    pub(super) references: usize,
}

pub(super) fn function_port_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("direction", "方向")
            .width(88)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("name", "端口标识").width(180),
        DataTableColumn::leaf("type", "值类型").width(160),
        DataTableColumn::leaf("references", "节点引用")
            .width(96)
            .align(DataTableAlign::Center),
        DataTableColumn::leaf("actions", "操作")
            .width(96)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}
