use std::collections::{BTreeMap, BTreeSet};

use az_admin_shell_core::identifier_from_title;

use crate::{
    EndpointInputLocation, FunctionNode, PageEndpointDefinition, SymbolId,
    endpoint_identifier_is_valid,
};

pub(crate) fn unique_identifier_from_title<'a>(
    title: &str,
    fallback: &str,
    existing: impl Iterator<Item = &'a str>,
) -> String {
    let existing = existing.collect::<BTreeSet<_>>();
    let generated = identifier_from_title(title);
    let base = if generated.is_empty() {
        fallback.to_owned()
    } else {
        generated
    };
    unique_identifier(&base, &existing)
}

pub(crate) fn next_function_node_name(nodes: &[FunctionNode]) -> String {
    let existing = nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect::<BTreeSet<_>>();
    unique_numbered_identifier("node", &existing)
}

pub(crate) fn next_endpoint_path_parameter_name<'a>(
    path: &str,
    existing: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let existing = existing.collect::<BTreeSet<_>>();
    endpoint_path_parameter_names(path)
        .into_iter()
        .find(|candidate| !existing.contains(candidate.as_str()))
}

pub(crate) fn synchronize_path_parameter_names(endpoint: &mut PageEndpointDefinition) {
    let placeholders = endpoint_path_parameter_names(&endpoint.path);
    for (input, name) in endpoint
        .inputs
        .iter_mut()
        .filter(|input| input.location == EndpointInputLocation::Path)
        .zip(placeholders)
    {
        input.name = name;
        input.required = true;
    }
}

pub(crate) fn normalize_endpoint_parameter_names(
    endpoint: &mut PageEndpointDefinition,
    stable_input_names: &BTreeMap<SymbolId, String>,
    stable_output_names: &BTreeMap<SymbolId, String>,
) {
    let mut used_inputs = stable_input_names
        .values()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut placeholders = endpoint_path_parameter_names(&endpoint.path).into_iter();
    for input in &mut endpoint.inputs {
        if input.location == EndpointInputLocation::Path {
            input.name = placeholders.next().unwrap_or_else(|| {
                unique_identifier_from_title(
                    &input.title,
                    "input",
                    used_inputs.iter().map(String::as_str),
                )
            });
            input.required = true;
        } else if let Some(name) = stable_input_names.get(&input.id) {
            input.name.clone_from(name);
        } else {
            input.name = unique_identifier_from_title(
                &input.title,
                "input",
                used_inputs.iter().map(String::as_str),
            );
        }
        used_inputs.insert(input.name.clone());
    }

    let mut used_outputs = stable_output_names
        .values()
        .cloned()
        .collect::<BTreeSet<_>>();
    for output in &mut endpoint.outputs {
        if let Some(name) = stable_output_names.get(&output.id) {
            output.name.clone_from(name);
        } else {
            output.name = unique_identifier_from_title(
                &output.title,
                "output",
                used_outputs.iter().map(String::as_str),
            );
        }
        used_outputs.insert(output.name.clone());
    }
}

fn endpoint_path_parameter_names(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut remaining = path;
    while let Some((_, suffix)) = remaining.split_once('{') {
        let Some((candidate, rest)) = suffix.split_once('}') else {
            break;
        };
        if endpoint_identifier_is_valid(candidate) && !names.iter().any(|name| name == candidate) {
            names.push(candidate.to_owned());
        }
        remaining = rest;
    }
    names
}

fn unique_identifier(base: &str, existing: &BTreeSet<&str>) -> String {
    if !existing.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|index| format!("{base}_{index}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .expect("标识应始终存在可用后缀")
}

fn unique_numbered_identifier(prefix: &str, existing: &BTreeSet<&str>) -> String {
    (1..)
        .map(|index| format!("{prefix}_{index}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .expect("序号标识应始终存在可用后缀")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DefinitionState, EndpointInputDefinition, FunctionNodeEditor, FunctionNodeKind, SymbolId,
        ValueType,
    };

    #[test]
    fn title_identifier_uses_pinyin_and_avoids_duplicates() {
        assert_eq!(
            unique_identifier_from_title("用户名称", "input", ["yong_hu_ming_cheng"].into_iter()),
            "yong_hu_ming_cheng_2"
        );
        assert_eq!(
            unique_identifier_from_title("……", "output", std::iter::empty()),
            "output"
        );
    }

    #[test]
    fn function_node_name_reuses_the_first_free_sequence() {
        let nodes = ["node_1", "node_3"]
            .into_iter()
            .map(|name| FunctionNode {
                id: SymbolId::new(),
                name: name.to_owned(),
                state: DefinitionState::Known,
                editor: FunctionNodeEditor::default(),
                kind: FunctionNodeKind::Return,
            })
            .collect::<Vec<_>>();

        assert_eq!(next_function_node_name(&nodes), "node_2");
    }

    #[test]
    fn path_parameters_follow_placeholders_without_manual_names() {
        let mut endpoint = PageEndpointDefinition {
            id: SymbolId::new(),
            title: String::new(),
            description: String::new(),
            state: DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Convention,
            method: crate::RestMethod::Get,
            path: "/users/{userId}/orders/{order_id}".to_owned(),
            inputs: vec![
                EndpointInputDefinition {
                    id: SymbolId::new(),
                    name: "old_user".to_owned(),
                    title: "用户".to_owned(),
                    location: EndpointInputLocation::Path,
                    value_type: ValueType::Text,
                    required: false,
                },
                EndpointInputDefinition {
                    id: SymbolId::new(),
                    name: "old_order".to_owned(),
                    title: "订单".to_owned(),
                    location: EndpointInputLocation::Path,
                    value_type: ValueType::Text,
                    required: false,
                },
            ],
            outputs: Vec::new(),
        };

        synchronize_path_parameter_names(&mut endpoint);

        assert_eq!(endpoint.inputs[0].name, "userId");
        assert_eq!(endpoint.inputs[1].name, "order_id");
        assert!(endpoint.inputs.iter().all(|input| input.required));
        assert_eq!(
            next_endpoint_path_parameter_name(&endpoint.path, ["userId"].into_iter()),
            Some("order_id".to_owned())
        );
    }

    #[test]
    fn endpoint_names_are_normalized_before_save() {
        let existing_id = SymbolId::new();
        let mut endpoint = PageEndpointDefinition {
            id: SymbolId::new(),
            title: String::new(),
            description: String::new(),
            state: DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Convention,
            method: crate::RestMethod::Post,
            path: "/users/{userId}".to_owned(),
            inputs: vec![
                EndpointInputDefinition {
                    id: existing_id,
                    name: "changed".to_owned(),
                    title: "已有字段新标题".to_owned(),
                    location: EndpointInputLocation::Query,
                    value_type: ValueType::Text,
                    required: false,
                },
                EndpointInputDefinition {
                    id: SymbolId::new(),
                    name: "temporary".to_owned(),
                    title: "用户".to_owned(),
                    location: EndpointInputLocation::Path,
                    value_type: ValueType::Text,
                    required: false,
                },
            ],
            outputs: vec![crate::EndpointOutputDefinition {
                id: SymbolId::new(),
                name: "temporary".to_owned(),
                title: "处理结果".to_owned(),
                value_type: ValueType::Text,
            }],
        };
        let stable_inputs = BTreeMap::from([(existing_id, "stable".to_owned())]);

        normalize_endpoint_parameter_names(&mut endpoint, &stable_inputs, &BTreeMap::new());

        assert_eq!(endpoint.inputs[0].name, "stable");
        assert_eq!(endpoint.inputs[1].name, "userId");
        assert!(endpoint.inputs[1].required);
        assert_eq!(endpoint.outputs[0].name, "chu_li_jie_guo");
    }

    #[test]
    fn studio_ui_does_not_accept_manual_identifiers() {
        let sources = [
            include_str!("ui/model_panel.rs"),
            include_str!("ui/field_dialog.rs"),
            include_str!("ui/query_dialog.rs"),
            include_str!("ui/function_definition_dialog.rs"),
            include_str!("ui/function_node_dialog.rs"),
            include_str!("ui/function_node_fields.rs"),
            include_str!("ui/function_edge_dialog.rs"),
            include_str!("ui/endpoint_editor.rs"),
            include_str!("ui/page_dialog.rs"),
            include_str!("ui/menu_dialog.rs"),
            include_str!("ui/permission_panel.rs"),
        ]
        .join("\n");
        for forbidden in [
            "aria_label: \"模型标识\"",
            "aria_label: \"字段标识\"",
            "aria_label: \"查询标识\"",
            "aria_label: \"函数标识\"",
            "aria_label: \"页面标识\"",
            "aria_label: \"菜单标识\"",
            "aria_label: \"权限标识\"",
            "aria_label: \"节点名称\"",
            "aria_label: \"入参名称\"",
            "aria_label: \"响应字段名称\"",
            "aria_label: \"连线起点端口\"",
            "aria_label: \"连线终点端口\"",
            "aria_label: \"能力标识\"",
            "aria_label: \"查询条件参数",
            "aria_label: \"关联查询参数",
            "node.name = event.value()",
            "input.name = event.value()",
            "output.name = event.value()",
        ] {
            assert!(
                !sources.contains(forbidden),
                "Studio UI 不得手工编辑标识: {forbidden}"
            );
        }
    }
}
