use std::collections::BTreeSet;

use dioxus::html::{
    geometry::WheelDelta,
    input_data::{MouseButton, keyboard_types::Key},
};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub const NODE_H: f64 = 72.0;
const VIEWPORT_W: f64 = 800.0;
const VIEWPORT_H: f64 = 450.0;
const MINI_W: f64 = 140.0;
const MINI_H: f64 = 90.0;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkflowNodeKind {
    Trigger,
    Data,
    Agent,
    Output,
}

impl WorkflowNodeKind {
    fn dot_class(&self) -> &'static str {
        match self {
            Self::Trigger => "bg-yellow-500",
            Self::Data => "bg-blue-500",
            Self::Agent => "bg-purple-500",
            Self::Output => "bg-green-500",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Trigger => "触发",
            Self::Data => "数据",
            Self::Agent => "能力",
            Self::Output => "输出",
        }
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum WorkflowEdgeStyle {
    Dashed,
    #[default]
    Solid,
    Dotted,
}

impl WorkflowEdgeStyle {
    fn dash_array(&self) -> &'static str {
        match self {
            Self::Dashed => "6 3",
            Self::Solid => "none",
            Self::Dotted => "2 3",
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub initial_x: f64,
    pub initial_y: f64,
    pub width: f64,
    pub has_target: bool,
    pub has_source: bool,
    pub label: String,
    pub description: String,
    pub kind: WorkflowNodeKind,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    pub style: WorkflowEdgeStyle,
    pub label: Option<String>,
}

#[derive(Clone, Copy)]
struct DragState {
    node_index: usize,
    mouse_x: f64,
    mouse_y: f64,
    start_x: f64,
    start_y: f64,
}

#[derive(Clone, Copy)]
struct PanState {
    mouse_x: f64,
    mouse_y: f64,
    start_x: f64,
    start_y: f64,
}

#[derive(Clone)]
struct ConnectionState {
    from: String,
    from_x: f64,
    from_y: f64,
    mouse_x: f64,
    mouse_y: f64,
}

pub struct WorkflowState {
    pub nodes: Signal<Vec<WorkflowNode>>,
    pub edges: Signal<Vec<WorkflowEdge>>,
    pub positions: Signal<Vec<(f64, f64)>>,
    pan: Signal<(f64, f64)>,
    zoom: Signal<f64>,
    selected_nodes: Signal<BTreeSet<usize>>,
    selected_edge: Signal<Option<usize>>,
    drag: Signal<Option<DragState>>,
    pan_drag: Signal<Option<PanState>>,
    connection: Signal<Option<ConnectionState>>,
    editing_node: Signal<Option<usize>>,
    edit_buffer: Signal<String>,
}

impl Copy for WorkflowState {}

impl Clone for WorkflowState {
    fn clone(&self) -> Self {
        *self
    }
}

impl PartialEq for WorkflowState {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes && self.edges == other.edges && self.positions == other.positions
    }
}

pub fn use_workflow(nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> WorkflowState {
    let positions = nodes
        .iter()
        .map(|node| (node.initial_x, node.initial_y))
        .collect::<Vec<_>>();
    WorkflowState {
        nodes: use_signal(|| nodes),
        edges: use_signal(|| edges),
        positions: use_signal(|| positions),
        pan: use_signal(|| (0.0, 0.0)),
        zoom: use_signal(|| 1.0),
        selected_nodes: use_signal(BTreeSet::new),
        selected_edge: use_signal(|| None),
        drag: use_signal(|| None),
        pan_drag: use_signal(|| None),
        connection: use_signal(|| None),
        editing_node: use_signal(|| None),
        edit_buffer: use_signal(String::new),
    }
}

impl WorkflowState {
    pub fn zoom_value(&self) -> f64 {
        *self.zoom.read()
    }

    pub fn zoom_step(&mut self, factor: f64) {
        let old_zoom = *self.zoom.read();
        let zoom = (old_zoom * factor).clamp(0.25, 3.0);
        let ratio = zoom / old_zoom;
        let (pan_x, pan_y) = *self.pan.read();
        self.pan.set((
            VIEWPORT_W / 2.0 - (VIEWPORT_W / 2.0 - pan_x) * ratio,
            VIEWPORT_H / 2.0 - (VIEWPORT_H / 2.0 - pan_y) * ratio,
        ));
        self.zoom.set(zoom);
    }

    pub fn fit_to_view(&mut self, viewport_w: f64, viewport_h: f64, node_h: f64) {
        let nodes = self.nodes.read();
        let positions = self.positions.read();
        if nodes.is_empty() {
            return;
        }
        let min_x = positions
            .iter()
            .map(|value| value.0)
            .fold(f64::INFINITY, f64::min);
        let min_y = positions
            .iter()
            .map(|value| value.1)
            .fold(f64::INFINITY, f64::min);
        let max_x = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| positions[index].0 + node.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = positions
            .iter()
            .map(|value| value.1 + node_h)
            .fold(f64::NEG_INFINITY, f64::max);
        let content_width = (max_x - min_x).max(1.0);
        let content_height = (max_y - min_y).max(1.0);
        let padding = 48.0;
        let zoom = ((viewport_w - padding * 2.0) / content_width)
            .min((viewport_h - padding * 2.0) / content_height)
            .clamp(0.25, 3.0);
        drop(positions);
        drop(nodes);
        self.zoom.set(zoom);
        self.pan.set((
            (viewport_w - content_width * zoom) / 2.0 - min_x * zoom,
            (viewport_h - content_height * zoom) / 2.0 - min_y * zoom,
        ));
    }

    fn transform(&self) -> String {
        let (x, y) = *self.pan.read();
        let zoom = *self.zoom.read();
        format!("translate({x:.2}px, {y:.2}px) scale({zoom:.4})")
    }

    fn position(&self, index: usize) -> (f64, f64) {
        self.positions
            .read()
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    fn select_node(&mut self, index: usize) {
        self.selected_nodes.write().clear();
        self.selected_nodes.write().insert(index);
        self.selected_edge.set(None);
    }

    fn deselect(&mut self) {
        self.selected_nodes.write().clear();
        self.selected_edge.set(None);
    }

    fn is_selected(&self, index: usize) -> bool {
        self.selected_nodes.read().contains(&index)
    }

    fn delete_node(&mut self, index: usize) {
        let Some(node) = self.nodes.read().get(index).cloned() else {
            return;
        };
        self.nodes.write().remove(index);
        self.positions.write().remove(index);
        self.edges
            .write()
            .retain(|edge| edge.from != node.id && edge.to != node.id);
        self.selected_nodes.write().clear();
    }

    fn delete_selected(&mut self) {
        let selected_edge = *self.selected_edge.read();
        if let Some(edge_index) = selected_edge {
            if edge_index < self.edges.read().len() {
                self.edges.write().remove(edge_index);
            }
            self.selected_edge.set(None);
            return;
        }
        let indices = self
            .selected_nodes
            .read()
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        for index in indices {
            self.delete_node(index);
        }
    }

    fn start_drag(&mut self, index: usize, mouse_x: f64, mouse_y: f64) {
        let (start_x, start_y) = self.position(index);
        self.drag.set(Some(DragState {
            node_index: index,
            mouse_x,
            mouse_y,
            start_x,
            start_y,
        }));
    }

    fn update_drag(&mut self, mouse_x: f64, mouse_y: f64) {
        let Some(drag) = *self.drag.read() else {
            return;
        };
        let zoom = *self.zoom.read();
        let x = (drag.start_x + (mouse_x - drag.mouse_x) / zoom).max(0.0);
        let y = (drag.start_y + (mouse_y - drag.mouse_y) / zoom).max(0.0);
        if let Some(position) = self.positions.write().get_mut(drag.node_index) {
            *position = (x, y);
        }
    }

    fn start_pan(&mut self, mouse_x: f64, mouse_y: f64) {
        let (start_x, start_y) = *self.pan.read();
        self.pan_drag.set(Some(PanState {
            mouse_x,
            mouse_y,
            start_x,
            start_y,
        }));
    }

    fn update_pan(&mut self, mouse_x: f64, mouse_y: f64) {
        let Some(pan) = *self.pan_drag.read() else {
            return;
        };
        self.pan.set((
            pan.start_x + mouse_x - pan.mouse_x,
            pan.start_y + mouse_y - pan.mouse_y,
        ));
    }

    fn stop_pointer(&mut self) {
        self.drag.set(None);
        self.pan_drag.set(None);
        self.connection.set(None);
    }

    fn start_connection(&mut self, node_id: String, x: f64, y: f64) {
        self.connection.set(Some(ConnectionState {
            from: node_id,
            from_x: x,
            from_y: y,
            mouse_x: x,
            mouse_y: y,
        }));
    }

    fn update_connection(&mut self, viewport_x: f64, viewport_y: f64) {
        let (pan_x, pan_y) = *self.pan.read();
        let zoom = *self.zoom.read();
        if let Some(connection) = self.connection.write().as_mut() {
            connection.mouse_x = (viewport_x - pan_x) / zoom;
            connection.mouse_y = (viewport_y - pan_y) / zoom;
        }
    }

    fn finish_connection(&mut self, to: String) {
        let connection = self.connection.read().clone();
        self.connection.set(None);
        let Some(connection) = connection else {
            return;
        };
        if connection.from == to {
            return;
        }
        let exists = self
            .edges
            .read()
            .iter()
            .any(|edge| edge.from == connection.from && edge.to == to);
        if !exists {
            self.edges.write().push(WorkflowEdge {
                from: connection.from,
                to,
                style: WorkflowEdgeStyle::Solid,
                label: None,
            });
        }
    }

    fn connection_path(&self) -> Option<String> {
        let connection = self.connection.read();
        let connection = connection.as_ref()?;
        Some(bezier_path(
            connection.from_x,
            connection.from_y,
            connection.mouse_x,
            connection.mouse_y,
        ))
    }

    fn start_edit(&mut self, index: usize) {
        let Some(label) = self.nodes.read().get(index).map(|node| node.label.clone()) else {
            return;
        };
        self.edit_buffer.set(label);
        self.editing_node.set(Some(index));
    }

    fn finish_edit(&mut self) {
        let Some(index) = *self.editing_node.read() else {
            return;
        };
        let label = self.edit_buffer.read().trim().to_owned();
        if !label.is_empty()
            && let Some(node) = self.nodes.write().get_mut(index)
        {
            node.label = label;
        }
        self.editing_node.set(None);
    }

    fn edge_paths(&self) -> Vec<String> {
        let nodes = self.nodes.read();
        let positions = self.positions.read();
        self.edges
            .read()
            .iter()
            .filter_map(|edge| {
                let from_index = nodes.iter().position(|node| node.id == edge.from)?;
                let to_index = nodes.iter().position(|node| node.id == edge.to)?;
                let from = &nodes[from_index];
                let (from_x, from_y) = positions[from_index];
                let (to_x, to_y) = positions[to_index];
                Some(bezier_path(
                    from_x + from.width,
                    from_y + NODE_H / 2.0,
                    to_x,
                    to_y + NODE_H / 2.0,
                ))
            })
            .collect()
    }
}

#[component]
pub fn WorkflowCanvas(
    state: WorkflowState,
    children: Element,
    #[props(optional)] overlay: Option<Element>,
) -> Element {
    let transform = state.transform();
    let paths = state.edge_paths();
    let connection_path = state.connection_path();
    let selected_edge = *state.selected_edge.read();
    let mut state = state;

    rsx! {
        div {
            class: "relative overflow-hidden rounded-md border bg-background outline-none select-none",
            style: "height:450px;cursor:grab;touch-action:none;",
            tabindex: "0",
            onkeydown: move |event| match event.data().key() {
                Key::Delete | Key::Backspace => {
                    event.prevent_default();
                    state.delete_selected();
                }
                Key::Escape => state.deselect(),
                _ => {}
            },
            onmousedown: move |event| {
                if event.data().trigger_button() == Some(MouseButton::Secondary) {
                    return;
                }
                let point = event.data().client_coordinates();
                state.finish_edit();
                state.deselect();
                state.start_pan(point.x, point.y);
            },
            onmousemove: move |event| {
                let client = event.data().client_coordinates();
                let element = event.data().element_coordinates();
                state.update_drag(client.x, client.y);
                state.update_pan(client.x, client.y);
                state.update_connection(element.x, element.y);
            },
            onmouseup: move |_| state.stop_pointer(),
            onmouseleave: move |_| state.stop_pointer(),
            onwheel: move |event| {
                event.prevent_default();
                let factor = match event.data().delta() {
                    WheelDelta::Pixels(value) if value.y < 0.0 => 1.1,
                    WheelDelta::Lines(value) if value.y < 0.0 => 1.1,
                    WheelDelta::Pages(value) if value.y < 0.0 => 1.1,
                    _ => 1.0 / 1.1,
                };
                state.zoom_step(factor);
            },
            div {
                style: "position:absolute;inset:0;width:3000px;height:2000px;transform:{transform};transform-origin:0 0;",
                div {
                    class: "pointer-events-none absolute inset-0 text-foreground",
                    style: "background-image:radial-gradient(circle,currentColor 1px,transparent 1px);background-size:20px 20px;opacity:.1;",
                }
                svg {
                    class: "pointer-events-none absolute inset-0 overflow-visible",
                    width: "3000",
                    height: "2000",
                    defs {
                        marker {
                            id: "aio-workflow-arrow",
                            marker_width: "8",
                            marker_height: "8",
                            ref_x: "7",
                            ref_y: "3",
                            orient: "auto",
                            path { d: "M0,0 L0,6 L8,3 z", fill: "currentColor", class: "text-muted-foreground" }
                        }
                    }
                    for (index, path) in paths.iter().enumerate() {
                        path {
                            d: path.as_str(),
                            fill: "none",
                            stroke: "currentColor",
                            class: if selected_edge == Some(index) { "pointer-events-auto text-primary" } else { "pointer-events-auto text-muted-foreground" },
                            stroke_width: if selected_edge == Some(index) { "2.5" } else { "2" },
                            stroke_linecap: "round",
                            stroke_dasharray: state.edges.read()[index].style.dash_array(),
                            marker_end: "url(#aio-workflow-arrow)",
                            style: "cursor:pointer;",
                            onclick: move |event| {
                                event.stop_propagation();
                                state.selected_nodes.write().clear();
                                state.selected_edge.set(Some(index));
                            }
                        }
                    }
                    if let Some(path) = connection_path {
                        path {
                            d: path,
                            fill: "none",
                            stroke: "currentColor",
                            class: "text-primary",
                            stroke_width: "2",
                            stroke_dasharray: "5 4",
                        }
                    }
                }
                {children}
            }
            {overlay}
        }
    }
}

#[component]
pub fn WorkflowNodeWrapper(state: WorkflowState, idx: usize, children: Element) -> Element {
    let Some(node) = state.nodes.read().get(idx).cloned() else {
        return rsx! {};
    };
    let (x, y) = state.position(idx);
    let selected = state.is_selected(idx);
    let editing = *state.editing_node.read() == Some(idx);
    let node_id = node.id.clone();
    let target_id = node.id.clone();
    let width = node.width;
    let mut state = state;

    rsx! {
        div {
            class: if selected { "absolute rounded-md ring-2 ring-primary ring-offset-2 ring-offset-background shadow-lg" } else { "absolute rounded-md shadow-sm" },
            style: "left:{x:.1}px;top:{y:.1}px;width:{width:.0}px;cursor:grab;",
            onmousedown: move |event| {
                event.stop_propagation();
                let point = event.data().client_coordinates();
                state.select_node(idx);
                state.start_drag(idx, point.x, point.y);
            },
            ondoubleclick: move |event| {
                event.stop_propagation();
                state.start_edit(idx);
            },
            {children}
            if selected {
                button {
                    class: "absolute right-1 top-1 z-10 flex size-5 items-center justify-center rounded text-xs text-muted-foreground hover:bg-destructive/10 hover:text-destructive",
                    title: "删除节点",
                    aria_label: "删除节点",
                    onclick: move |event| {
                        event.stop_propagation();
                        state.delete_node(idx);
                    },
                    "×"
                }
            }
            if editing {
                div {
                    class: "absolute inset-0 z-20 flex items-center rounded-md bg-background px-3 ring-2 ring-primary",
                    onmousedown: move |event| event.stop_propagation(),
                    input {
                        class: "w-full bg-transparent text-sm font-medium outline-none",
                        value: state.edit_buffer.read().clone(),
                        oninput: move |event| state.edit_buffer.set(event.value()),
                        onkeydown: move |event| match event.data().key() {
                            Key::Enter => state.finish_edit(),
                            Key::Escape => state.editing_node.set(None),
                            _ => {}
                        },
                        onblur: move |_| state.finish_edit(),
                    }
                }
            }
            if node.has_source {
                div {
                    class: "absolute right-0 top-1/2 z-10 size-3 rounded-full border-2 border-primary bg-background",
                    style: "transform:translate(50%,-50%);cursor:crosshair;",
                    "data-testid": "source-handle",
                    onmousedown: move |event| {
                        event.stop_propagation();
                        state.start_connection(node_id.clone(), x + width, y + NODE_H / 2.0);
                    }
                }
            }
            if node.has_target {
                div {
                    class: "absolute left-0 top-1/2 z-10 size-3 rounded-full border-2 border-primary bg-background",
                    style: "transform:translate(-50%,-50%);cursor:crosshair;",
                    "data-testid": "target-handle",
                    onmouseup: move |event| {
                        event.stop_propagation();
                        state.finish_connection(target_id.clone());
                    }
                }
            }
        }
    }
}

#[component]
pub fn WorkflowDefaultNode(node: WorkflowNode) -> Element {
    rsx! {
        article { class: "h-[72px] overflow-hidden rounded-md border bg-card text-card-foreground shadow-sm",
            header { class: "flex h-9 items-center gap-2 border-b bg-secondary/50 px-3",
                span { class: "size-2 shrink-0 rounded-full {node.kind.dot_class()}" }
                strong { class: "min-w-0 flex-1 truncate text-xs font-medium", "{node.label}" }
                span { class: "shrink-0 text-[10px] text-muted-foreground", "{node.kind.label()}" }
            }
            div { class: "truncate px-3 py-2 font-mono text-[10px] text-muted-foreground", "{node.description}" }
        }
    }
}

#[component]
pub fn WorkflowMinimap(state: WorkflowState) -> Element {
    let nodes = state.nodes.read().clone();
    let positions = state.positions.read().clone();
    let edges = state.edges.read().clone();
    let scale_x = MINI_W / 1600.0;
    let scale_y = MINI_H / 900.0;
    let node_rects = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            (
                positions[index].0 * scale_x,
                positions[index].1 * scale_y,
                node.width * scale_x,
                NODE_H * scale_y,
            )
        })
        .collect::<Vec<_>>();
    let mut state = state;

    rsx! {
        div {
            class: "cursor-pointer overflow-hidden rounded-md border bg-background/90 shadow-sm backdrop-blur-sm",
            style: "position:absolute;right:12px;bottom:12px;width:{MINI_W}px;height:{MINI_H}px;",
            onclick: move |event| {
                let point = event.data().element_coordinates();
                let zoom = state.zoom_value();
                state.pan.set((
                    VIEWPORT_W / 2.0 - point.x / scale_x * zoom,
                    VIEWPORT_H / 2.0 - point.y / scale_y * zoom,
                ));
            },
            svg { width: "{MINI_W}", height: "{MINI_H}",
                for edge in edges {
                    if let (Some(from), Some(to)) = (
                        nodes.iter().position(|node| node.id == edge.from),
                        nodes.iter().position(|node| node.id == edge.to),
                    ) {
                        path {
                            d: bezier_path(
                                (positions[from].0 + nodes[from].width) * scale_x,
                                (positions[from].1 + NODE_H / 2.0) * scale_y,
                                positions[to].0 * scale_x,
                                (positions[to].1 + NODE_H / 2.0) * scale_y,
                            ),
                            fill: "none",
                            stroke: "currentColor",
                            class: "text-border",
                            stroke_width: "1",
                        }
                    }
                }
                for (x, y, width, height) in node_rects {
                    rect {
                        x: "{x:.1}",
                        y: "{y:.1}",
                        width: "{width:.1}",
                        height: "{height:.1}",
                        rx: "2",
                        fill: "currentColor",
                        class: "text-muted-foreground",
                        opacity: ".45",
                    }
                }
            }
        }
    }
}

fn bezier_path(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> String {
    let offset = ((to_x - from_x).abs() / 2.0).clamp(40.0, 90.0);
    format!(
        "M {from_x:.1} {from_y:.1} C {:.1} {from_y:.1}, {:.1} {to_y:.1}, {to_x:.1} {to_y:.1}",
        from_x + offset,
        to_x - offset,
    )
}
