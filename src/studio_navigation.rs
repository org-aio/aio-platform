use std::collections::{BTreeMap, BTreeSet};

use crate::{
    FunctionDefinition, FunctionNodeKind, GraphPatch, MenuDefinition, PageRendererDefinition,
    ProgramDefinition, RouteDefinition, SymbolId, ValueType,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ModelUsageSummary {
    pub(crate) model_fields: usize,
    pub(crate) page_layouts: usize,
    pub(crate) page_endpoints: usize,
    pub(crate) functions: usize,
}

#[cfg_attr(not(feature = "runtime-ui"), allow(dead_code))]
impl ModelUsageSummary {
    pub(crate) const fn total(self) -> usize {
        self.model_fields + self.page_layouts + self.page_endpoints + self.functions
    }

    pub(crate) fn description(self) -> String {
        let mut parts = Vec::new();
        if self.model_fields > 0 {
            parts.push(format!("{} 个模型字段", self.model_fields));
        }
        if self.page_layouts > 0 {
            parts.push(format!("{} 个页面布局", self.page_layouts));
        }
        if self.page_endpoints > 0 {
            parts.push(format!("{} 个页面接口", self.page_endpoints));
        }
        if self.functions > 0 {
            parts.push(format!("{} 个函数", self.functions));
        }
        parts.join("、")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(not(feature = "runtime-ui"), allow(dead_code))]
pub(crate) enum StudioTab {
    #[default]
    Applications,
    Models,
    Pages,
    Functions,
    Endpoints,
    Menus,
    Permissions,
}

pub(crate) fn definition_matches_search(name: &str, title: &str, normalized_search: &str) -> bool {
    normalized_search.is_empty()
        || name.to_lowercase().contains(normalized_search)
        || title.to_lowercase().contains(normalized_search)
}

pub(crate) fn preferred_draft_scene_id(
    scene_ids: &[SymbolId],
    current: Option<SymbolId>,
    published: Option<SymbolId>,
) -> Option<SymbolId> {
    published
        .filter(|scene_id| scene_ids.contains(scene_id))
        .or_else(|| current.filter(|scene_id| scene_ids.contains(scene_id)))
        .or_else(|| scene_ids.first().copied())
}

pub(crate) fn page_menu_reference_count(menus: &[MenuDefinition], page_id: SymbolId) -> usize {
    menus
        .iter()
        .map(|menu| {
            usize::from(menu.page_id == Some(page_id))
                + page_menu_reference_count(&menu.children, page_id)
        })
        .sum()
}

pub(crate) fn permission_usage_map(definition: &ProgramDefinition) -> BTreeMap<SymbolId, usize> {
    let mut usages = definition
        .permissions
        .iter()
        .map(|permission| (permission.id, 0))
        .collect::<BTreeMap<_, _>>();
    for menu in &definition.menus {
        collect_menu_permission_usage(menu, &mut usages);
    }
    for route in &definition.routes {
        for permission_id in &route.required_permissions {
            increment_permission_usage(&mut usages, *permission_id);
        }
    }
    for function in &definition.functions {
        for permission_id in &function.required_permissions {
            increment_permission_usage(&mut usages, *permission_id);
        }
    }
    usages
}

fn collect_menu_permission_usage(menu: &MenuDefinition, usages: &mut BTreeMap<SymbolId, usize>) {
    for permission_id in &menu.required_permissions {
        increment_permission_usage(usages, *permission_id);
    }
    for access in [
        &menu.row_actions.detail,
        &menu.row_actions.edit,
        &menu.row_actions.delete,
    ] {
        if let crate::MenuActionAccess::Permission { permission_id } = access {
            increment_permission_usage(usages, *permission_id);
        }
    }
    for child in &menu.children {
        collect_menu_permission_usage(child, usages);
    }
}

fn increment_permission_usage(usages: &mut BTreeMap<SymbolId, usize>, permission_id: SymbolId) {
    if let Some(count) = usages.get_mut(&permission_id) {
        *count = count.saturating_add(1);
    }
}

pub(crate) fn function_reference_count(
    definition: &ProgramDefinition,
    function_id: SymbolId,
) -> usize {
    definition
        .functions
        .iter()
        .filter(|function| function.id != function_id)
        .flat_map(|function| &function.graph.nodes)
        .filter(|node| {
            matches!(
                &node.kind,
                FunctionNodeKind::ForEach {
                    body_function_id,
                    ..
                } if *body_function_id == function_id
            )
        })
        .count()
}

pub(crate) fn function_port_reference_count(
    function: &FunctionDefinition,
    port_id: SymbolId,
) -> usize {
    function
        .graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                &node.kind,
                FunctionNodeKind::Input {
                    port_id: referenced_port_id,
                } | FunctionNodeKind::Output {
                    port_id: referenced_port_id,
                } if *referenced_port_id == port_id
            )
        })
        .count()
}

pub(crate) fn function_node_reference_count(
    function: &FunctionDefinition,
    node_id: SymbolId,
) -> usize {
    function
        .graph
        .nodes
        .iter()
        .filter(|node| match &node.kind {
            FunctionNodeKind::Object { fields } => fields.values().any(|value| *value == node_id),
            FunctionNodeKind::List { items } => items.contains(&node_id),
            FunctionNodeKind::FieldAccess { object, .. } => *object == node_id,
            FunctionNodeKind::Format { values, .. } => values.contains(&node_id),
            _ => false,
        })
        .count()
}

pub(crate) fn model_usage_summary(
    definition: &ProgramDefinition,
    model_id: SymbolId,
) -> ModelUsageSummary {
    let model_field_ids = definition
        .models
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| {
            model
                .fields
                .iter()
                .map(|field| field.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let model_fields = definition
        .models
        .iter()
        .filter(|model| model.id != model_id)
        .flat_map(|model| &model.fields)
        .filter(|field| {
            value_type_references_model(&field.value_type, model_id)
                || field
                    .relation
                    .as_ref()
                    .is_some_and(|relation| relation.target_model_id == model_id)
        })
        .count();
    let page_layouts = definition
        .pages
        .iter()
        .filter(|page| match &page.renderer {
            PageRendererDefinition::ConventionFile | PageRendererDefinition::MenuTree => false,
            PageRendererDefinition::CrudTable { table } => table.model_id == Some(model_id),
            PageRendererDefinition::TreeTable { tree, table } => {
                table.model_id == Some(model_id)
                    || tree.model_id == Some(model_id)
                    || [
                        tree.label_field_id,
                        tree.parent_field_id,
                        tree.table_relation_field_id,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|field_id| model_field_ids.contains(&field_id))
            }
        })
        .count();
    let page_endpoints = definition
        .pages
        .iter()
        .flat_map(|page| &page.endpoints)
        .filter(|endpoint| {
            endpoint
                .inputs
                .iter()
                .map(|input| &input.value_type)
                .chain(endpoint.outputs.iter().map(|output| &output.value_type))
                .any(|value_type| value_type_references_model(value_type, model_id))
        })
        .count();
    let functions = definition
        .functions
        .iter()
        .filter(|function| function_references_model(function, model_id, &model_field_ids))
        .count();

    ModelUsageSummary {
        model_fields,
        page_layouts,
        page_endpoints,
        functions,
    }
}

fn value_type_references_model(value_type: &ValueType, model_id: SymbolId) -> bool {
    match value_type {
        ValueType::Object {
            model_id: referenced_model_id,
        } => *referenced_model_id == model_id,
        ValueType::List { item } => value_type_references_model(item, model_id),
        ValueType::Optional { value } => value_type_references_model(value, model_id),
        ValueType::Any
        | ValueType::Null
        | ValueType::Boolean
        | ValueType::Integer
        | ValueType::Decimal
        | ValueType::Text
        | ValueType::TimestampMs
        | ValueType::File => false,
    }
}

fn function_references_model(
    function: &FunctionDefinition,
    model_id: SymbolId,
    model_field_ids: &[SymbolId],
) -> bool {
    function
        .inputs
        .iter()
        .chain(&function.outputs)
        .any(|port| value_type_references_model(&port.value_type, model_id))
        || function.graph.nodes.iter().any(|node| match &node.kind {
            FunctionNodeKind::Constant { value_type, .. } => {
                value_type_references_model(value_type, model_id)
            }
            FunctionNodeKind::CreateRecord {
                model_id: referenced_model_id,
            }
            | FunctionNodeKind::ReadRecord {
                model_id: referenced_model_id,
            }
            | FunctionNodeKind::UpdateRecord {
                model_id: referenced_model_id,
            }
            | FunctionNodeKind::DeleteRecord {
                model_id: referenced_model_id,
            }
            | FunctionNodeKind::QueryRecords {
                model_id: referenced_model_id,
                ..
            } => *referenced_model_id == model_id,
            FunctionNodeKind::Object { fields } => fields
                .keys()
                .any(|field_id| model_field_ids.contains(field_id)),
            FunctionNodeKind::FieldAccess { field_id, .. } => model_field_ids.contains(field_id),
            FunctionNodeKind::ValidateForm { rules } => rules
                .iter()
                .any(|rule| model_field_ids.contains(&rule.field_id)),
            _ => false,
        })
}

pub(crate) fn delete_page_patches(
    routes: &[RouteDefinition],
    page_id: SymbolId,
) -> Vec<GraphPatch> {
    routes
        .iter()
        .filter(|route| route.page_id == page_id)
        .map(|route| GraphPatch::Delete {
            target_id: route.id,
        })
        .chain(std::iter::once(GraphPatch::Delete { target_id: page_id }))
        .collect()
}

pub(crate) fn delete_menu_patches(
    menus: &[MenuDefinition],
    routes: &[RouteDefinition],
    target_id: SymbolId,
) -> Vec<GraphPatch> {
    let Some(target) = find_menu(menus, target_id) else {
        return vec![GraphPatch::Delete { target_id }];
    };
    let mut page_ids = BTreeSet::new();
    collect_menu_page_ids(target, &mut page_ids);
    let mut patches = vec![GraphPatch::Delete { target_id }];

    for page_id in page_ids {
        if menus
            .iter()
            .any(|menu| menu_references_page_outside(menu, target_id, page_id))
        {
            continue;
        }
        patches.extend(
            routes
                .iter()
                .filter(|route| route.page_id == page_id)
                .map(|route| GraphPatch::Delete {
                    target_id: route.id,
                }),
        );
        patches.push(GraphPatch::Delete { target_id: page_id });
    }

    patches
}

fn find_menu(menus: &[MenuDefinition], target_id: SymbolId) -> Option<&MenuDefinition> {
    menus.iter().find_map(|menu| {
        if menu.id == target_id {
            Some(menu)
        } else {
            find_menu(&menu.children, target_id)
        }
    })
}

fn collect_menu_page_ids(menu: &MenuDefinition, page_ids: &mut BTreeSet<SymbolId>) {
    if let Some(page_id) = menu.page_id {
        page_ids.insert(page_id);
    }
    for child in &menu.children {
        collect_menu_page_ids(child, page_ids);
    }
}

fn menu_references_page_outside(
    menu: &MenuDefinition,
    excluded_menu_id: SymbolId,
    page_id: SymbolId,
) -> bool {
    if menu.id == excluded_menu_id {
        return false;
    }
    menu.page_id == Some(page_id)
        || menu
            .children
            .iter()
            .any(|child| menu_references_page_outside(child, excluded_menu_id, page_id))
}

#[cfg(test)]
#[path = "studio_navigation_tests.rs"]
mod tests;
