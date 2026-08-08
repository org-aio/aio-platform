use super::*;

#[component]
pub(super) fn FunctionPortsPanel(
    function: FunctionDefinition,
    mut editor: Signal<Option<FunctionPortEditorTarget>>,
    mut deleting: Signal<Option<DefinitionDeleteTarget>>,
) -> Element {
    let rows = function
        .inputs
        .iter()
        .cloned()
        .map(|port| FunctionPortRow {
            references: function_port_reference_count(&function, port.id),
            direction: FunctionPortDirection::Input,
            port,
        })
        .chain(
            function
                .outputs
                .iter()
                .cloned()
                .map(|port| FunctionPortRow {
                    references: function_port_reference_count(&function, port.id),
                    direction: FunctionPortDirection::Output,
                    port,
                }),
        )
        .collect::<Vec<_>>();

    rsx! {
        section { class: "aio-function-table-panel",
            header {
                div {
                    h3 { "函数端口" }
                    p { "{function.inputs.len()} 个输入 · {function.outputs.len()} 个输出" }
                }
                div { class: "aio-function-table-panel__actions",
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| editor.set(Some(FunctionPortEditorTarget::CreateInput)),
                        icons::Plus { class: "size-4" }
                        "输入"
                    }
                    Button {
                        r#type: "button",
                        size: ButtonSize::Sm,
                        variant: ButtonVariant::Outline,
                        onclick: move |_| editor.set(Some(FunctionPortEditorTarget::CreateOutput)),
                        icons::Plus { class: "size-4" }
                        "输出"
                    }
                }
            }
            div { class: "aio-function-table-panel__body",
                DataTable::<FunctionPortRow> {
                    class: "aio-function-data-table",
                    aria_label: "函数输入输出端口",
                    rows,
                    columns: function_port_columns(),
                    max_height: "100%",
                    empty_text: "暂无函数端口".to_owned(),
                    row_key: |row: FunctionPortRow| row.port.id.to_string(),
                    render_cell: move |cell: DataTableCellContext<FunctionPortRow>| {
                        let row = cell.row;
                        match cell.column.key.as_str() {
                            "direction" => rsx! { Badge {
                                variant: BadgeVariant::Outline,
                                "{row.direction.label()}"
                            } },
                            "name" => rsx! { code { "{row.port.name}" } },
                            "type" => rsx! { "{value_type_label(&row.port.value_type)}" },
                            "references" => rsx! { "{row.references}" },
                            "actions" => {
                                let edit_target = match row.direction {
                                    FunctionPortDirection::Input => {
                                        FunctionPortEditorTarget::EditInput(row.port.id)
                                    }
                                    FunctionPortDirection::Output => {
                                        FunctionPortEditorTarget::EditOutput(row.port.id)
                                    }
                                };
                                let delete_title = if row.references > 0 {
                                    format!("该端口被 {} 个节点引用，不能删除", row.references)
                                } else {
                                    format!("删除端口 {}", row.port.name)
                                };
                                rsx! {
                                    div { class: "aio-function-table-panel__row-actions",
                                        Button {
                                            r#type: "button",
                                            size: ButtonSize::IconXs,
                                            variant: ButtonVariant::Ghost,
                                            title: "编辑端口 {row.port.name}",
                                            aria_label: "编辑端口 {row.port.name}",
                                            onclick: move |_| editor.set(Some(edit_target)),
                                            icons::Pencil { class: "size-3" }
                                        }
                                        Button {
                                            r#type: "button",
                                            size: ButtonSize::IconXs,
                                            variant: ButtonVariant::Ghost,
                                            disabled: row.references > 0,
                                            title: delete_title.clone(),
                                            aria_label: delete_title,
                                            onclick: {
                                                let port_id = row.port.id;
                                                let port_name = row.port.name.clone();
                                                move |_| deleting.set(Some(DefinitionDeleteTarget {
                                                    id: port_id,
                                                    kind: "函数端口",
                                                    label: port_name.clone(),
                                                }))
                                            },
                                            icons::Trash2 { class: "size-3" }
                                        }
                                    }
                                }
                            },
                            _ => rsx! { "—" },
                        }
                    },
                }
            }
        }
    }
}

pub(super) fn function_edge_columns() -> Vec<DataTableColumn> {
    vec![
        DataTableColumn::leaf("from", "起点节点")
            .width(180)
            .fixed(DataTableFixed::Left),
        DataTableColumn::leaf("from_port", "起点端口").width(130),
        DataTableColumn::leaf("to", "终点节点").width(180),
        DataTableColumn::leaf("to_port", "终点端口").width(130),
        DataTableColumn::leaf("actions", "操作")
            .width(72)
            .align(DataTableAlign::End)
            .fixed(DataTableFixed::Right),
    ]
}

#[component]
pub(super) fn FunctionEdgesPanel(
    function: FunctionDefinition,
    mut creating: Signal<bool>,
    mut deleting: Signal<Option<GraphEdge>>,
) -> Element {
    let nodes = function.graph.nodes.clone();
    rsx! {
        section { class: "aio-function-table-panel",
            header {
                div {
                    h3 { "节点连线" }
                    p { "{function.graph.edges.len()} 条结构化连线" }
                }
                Button {
                    r#type: "button",
                    size: ButtonSize::Sm,
                    variant: ButtonVariant::Outline,
                    disabled: nodes.len() < 2,
                    title: if nodes.len() < 2 { "至少需要两个节点" } else { "新建节点连线" },
                    onclick: move |_| creating.set(true),
                    icons::Plus { class: "size-4" }
                    "新建连线"
                }
            }
            div { class: "aio-function-table-panel__body",
                DataTable::<GraphEdge> {
                    class: "aio-function-data-table",
                    aria_label: "函数节点连线",
                    rows: function.graph.edges.clone(),
                    columns: function_edge_columns(),
                    max_height: "100%",
                    empty_text: "暂无节点连线".to_owned(),
                    row_key: |edge: GraphEdge| edge.id.to_string(),
                    render_cell: move |cell: DataTableCellContext<GraphEdge>| {
                        let edge = cell.row;
                        match cell.column.key.as_str() {
                            "from" => rsx! { "{function_node_name(&nodes, edge.from_node)}" },
                            "from_port" => rsx! { code { "{edge.from_port}" } },
                            "to" => rsx! { "{function_node_name(&nodes, edge.to_node)}" },
                            "to_port" => rsx! { code { "{edge.to_port}" } },
                            "actions" => rsx! {
                                Button {
                                    r#type: "button",
                                    size: ButtonSize::IconXs,
                                    variant: ButtonVariant::Ghost,
                                    title: "删除连线",
                                    aria_label: "删除连线",
                                    onclick: move |_| deleting.set(Some(edge.clone())),
                                    icons::Trash2 { class: "size-3" }
                                }
                            },
                            _ => rsx! { "—" },
                        }
                    },
                }
            }
        }
    }
}

pub(super) fn function_node_name(nodes: &[FunctionNode], node_id: SymbolId) -> &str {
    nodes
        .iter()
        .find(|node| node.id == node_id)
        .map(|node| node.name.as_str())
        .unwrap_or("未知节点")
}

pub(super) fn function_node_kind_title(kind: &FunctionNodeKind) -> &'static str {
    match kind {
        FunctionNodeKind::Constant { .. } => "常量",
        FunctionNodeKind::Input { .. } => "输入",
        FunctionNodeKind::Output { .. } => "输出",
        FunctionNodeKind::Object { .. } => "对象",
        FunctionNodeKind::List { .. } => "列表",
        FunctionNodeKind::FieldAccess { .. } => "字段读取",
        FunctionNodeKind::Format { .. } => "格式化",
        FunctionNodeKind::Compare { .. } => "比较",
        FunctionNodeKind::Boolean { .. } => "布尔",
        FunctionNodeKind::Math { .. } => "数学",
        FunctionNodeKind::Condition => "条件",
        FunctionNodeKind::ForEach { .. } => "遍历",
        FunctionNodeKind::ValidateForm { .. } => "表单校验",
        FunctionNodeKind::CreateRecord { .. } => "新增记录",
        FunctionNodeKind::ReadRecord { .. } => "读取记录",
        FunctionNodeKind::UpdateRecord { .. } => "更新记录",
        FunctionNodeKind::DeleteRecord { .. } => "删除记录",
        FunctionNodeKind::QueryRecords { .. } => "查询记录",
        FunctionNodeKind::Navigate { .. } => "页面跳转",
        FunctionNodeKind::Confirm { .. } => "确认",
        FunctionNodeKind::Notify { .. } => "通知",
        FunctionNodeKind::Return => "返回",
        FunctionNodeKind::Fail { .. } => "失败",
        FunctionNodeKind::Capability { .. } => "能力调用",
    }
}

pub(super) fn function_node_detail(
    kind: &FunctionNodeKind,
    models: &[ModelDefinition],
    routes: &[RouteDefinition],
) -> String {
    let model_title = |model_id: SymbolId| {
        models
            .iter()
            .find(|model| model.id == model_id)
            .map(|model| model.title.clone())
            .unwrap_or_else(|| "未知模型".to_owned())
    };
    match kind {
        FunctionNodeKind::Constant { value_type, .. } => value_type_label(value_type).to_owned(),
        FunctionNodeKind::Input { .. } | FunctionNodeKind::Output { .. } => "函数端口".to_owned(),
        FunctionNodeKind::Object { fields } => format!("{} 个字段", fields.len()),
        FunctionNodeKind::List { items } => format!("{} 个元素", items.len()),
        FunctionNodeKind::FieldAccess { .. } => "读取对象字段".to_owned(),
        FunctionNodeKind::Format { template, .. } => template.clone(),
        FunctionNodeKind::Compare { operator } => format!("{operator:?}"),
        FunctionNodeKind::Boolean { operator } => format!("{operator:?}"),
        FunctionNodeKind::Math { operator } => format!("{operator:?}"),
        FunctionNodeKind::Condition => "条件分支".to_owned(),
        FunctionNodeKind::ForEach { max_items, .. } => format!("最多 {max_items} 项"),
        FunctionNodeKind::ValidateForm { rules } => format!("{} 条规则", rules.len()),
        FunctionNodeKind::CreateRecord { model_id }
        | FunctionNodeKind::ReadRecord { model_id }
        | FunctionNodeKind::UpdateRecord { model_id }
        | FunctionNodeKind::DeleteRecord { model_id }
        | FunctionNodeKind::QueryRecords { model_id, .. } => model_title(*model_id),
        FunctionNodeKind::Navigate { route_id } => routes
            .iter()
            .find(|route| route.id == *route_id)
            .map(|route| route.path.clone())
            .unwrap_or_else(|| "未知路由".to_owned()),
        FunctionNodeKind::Confirm { .. } => "等待用户确认".to_owned(),
        FunctionNodeKind::Notify { level } => format!("{level:?}"),
        FunctionNodeKind::Return => "结束函数".to_owned(),
        FunctionNodeKind::Fail { code } => code.clone(),
        FunctionNodeKind::Capability {
            capability_id,
            operation,
        } => format!("{capability_id} · {operation}"),
    }
}

pub(super) fn function_permission_input_name(permission_id: SymbolId) -> String {
    format!("function_permission_{permission_id}")
}

pub(super) fn function_permissions_from_form(
    event: &FormEvent,
    permissions: &[PermissionDefinition],
) -> Vec<SymbolId> {
    permissions
        .iter()
        .filter(|permission| {
            !form_text(event, &function_permission_input_name(permission.id)).is_empty()
        })
        .map(|permission| permission.id)
        .collect()
}
