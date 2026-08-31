use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ApplicationTarget, FieldDefinition, FunctionDefinition, FunctionNode, GraphEdge,
    MenuDefinition, ModelAuditDefinition, ModelDefinition, ModelIndexDefinition,
    ModelPrimaryKeyDefinition, ModelQueryDefinition, ModelValidationDefinition, PageDefinition,
    PageEndpointDefinition, PermissionDefinition, PortDefinition, ProgramDefinition,
    RouteDefinition, SymbolId,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOrigin {
    Studio,
    Vibe,
    Migration,
}

/// 手工拖拽与 AI 共用的唯一变更协议。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphPatchBatch {
    pub base_version: i64,
    pub patches: Vec<GraphPatch>,
    pub origin: PatchOrigin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphPatch {
    Insert {
        parent_id: SymbolId,
        collection: ChildCollection,
        index: usize,
        entity: Box<GraphEntity>,
    },
    Delete {
        target_id: SymbolId,
    },
    Move {
        target_id: SymbolId,
        parent_id: SymbolId,
        collection: ChildCollection,
        index: usize,
    },
    Reorder {
        parent_id: SymbolId,
        collection: ChildCollection,
        ordered_ids: Vec<SymbolId>,
    },
    Rename {
        target_id: SymbolId,
        name: String,
        title: Option<String>,
    },
    SetProperty {
        target_id: SymbolId,
        property: EditableProperty,
        value: Value,
    },
    Connect {
        function_id: SymbolId,
        edge: GraphEdge,
    },
    Disconnect {
        function_id: SymbolId,
        edge_id: SymbolId,
    },
}

impl GraphPatch {
    #[must_use]
    pub fn insert(
        parent_id: SymbolId,
        collection: ChildCollection,
        index: usize,
        entity: GraphEntity,
    ) -> Self {
        Self::Insert {
            parent_id,
            collection,
            index,
            entity: Box::new(entity),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildCollection {
    Menus,
    MenuChildren,
    Models,
    Fields,
    ModelIndexes,
    ModelQueries,
    ModelValidations,
    Pages,
    PageEndpoints,
    Functions,
    FunctionInputs,
    FunctionOutputs,
    FunctionNodes,
    Routes,
    Permissions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GraphEntity {
    Menu(MenuDefinition),
    Model(ModelDefinition),
    Field(FieldDefinition),
    ModelIndex(ModelIndexDefinition),
    ModelQuery(ModelQueryDefinition),
    ModelValidation(ModelValidationDefinition),
    Page(PageDefinition),
    PageEndpoint(PageEndpointDefinition),
    Function(FunctionDefinition),
    Port(PortDefinition),
    FunctionNode(FunctionNode),
    Route(RouteDefinition),
    Permission(PermissionDefinition),
}

impl GraphEntity {
    #[must_use]
    pub fn id(&self) -> SymbolId {
        match self {
            Self::Menu(value) => value.id,
            Self::Model(value) => value.id,
            Self::Field(value) => value.id,
            Self::ModelIndex(value) => value.id,
            Self::ModelQuery(value) => value.id,
            Self::ModelValidation(value) => value.id,
            Self::Page(value) => value.id,
            Self::PageEndpoint(value) => value.id,
            Self::Function(value) => value.id,
            Self::Port(value) => value.id,
            Self::FunctionNode(value) => value.id,
            Self::Route(value) => value.id,
            Self::Permission(value) => value.id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum EditableProperty {
    Title,
    ApplicationTargets,
    RoutePath,
    RoutePermissions,
    Icon,
    MenuPage,
    MenuEnabled,
    MenuPermissions,
    MenuRowActions,
    PermissionEffects,
    FunctionPermissions,
    PageRenderer,
    PageEndpoint,
    DefinitionState,
    FieldRequired,
    FieldListVisible,
    FieldDetailVisible,
    FieldFormVisible,
    FieldFormEditable,
    FieldFilterable,
    FieldSortable,
    FieldUnique,
    FieldValueType,
    FieldRelation,
    FieldOptions,
    ModelIndexFields,
    ModelIndexUnique,
    ModelQuery,
    ModelValidation,
    ModelPrimaryKey,
    ModelAudit,
    FunctionPort,
    FunctionNode,
    FunctionNodePosition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchError {
    DuplicateSymbol(SymbolId),
    TargetNotFound(SymbolId),
    ParentNotFound(SymbolId),
    CollectionMismatch,
    InvalidValue(String),
    InvalidOrder,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateSymbol(id) => write!(formatter, "符号重复: {id}"),
            Self::TargetNotFound(id) => write!(formatter, "目标不存在: {id}"),
            Self::ParentNotFound(id) => write!(formatter, "父级不存在: {id}"),
            Self::CollectionMismatch => formatter.write_str("实体与目标集合不匹配"),
            Self::InvalidValue(message) => write!(formatter, "属性值无效: {message}"),
            Self::InvalidOrder => formatter.write_str("重排列表必须与现有子项完全一致"),
        }
    }
}

impl std::error::Error for PatchError {}

impl ProgramDefinition {
    pub fn apply_patch_batch(&mut self, batch: &GraphPatchBatch) -> Result<(), PatchError> {
        let mut next = self.clone();
        for patch in &batch.patches {
            next.apply_patch_mut(patch)?;
        }
        *self = next;
        Ok(())
    }

    pub fn apply_patch(&mut self, patch: &GraphPatch) -> Result<(), PatchError> {
        let mut next = self.clone();
        next.apply_patch_mut(patch)?;
        *self = next;
        Ok(())
    }

    fn apply_patch_mut(&mut self, patch: &GraphPatch) -> Result<(), PatchError> {
        match patch {
            GraphPatch::Insert {
                parent_id,
                collection,
                index,
                entity,
            } => {
                ensure_studio_insertable(entity)?;
                self.insert_entity(*parent_id, *collection, *index, entity.as_ref().clone())
            }
            GraphPatch::Delete { target_id } => self.delete_entity(*target_id),
            GraphPatch::Move {
                target_id,
                parent_id,
                collection,
                index,
            } => {
                let entity = self.take_entity(*target_id)?;
                self.insert_entity(*parent_id, *collection, *index, entity)
            }
            GraphPatch::Reorder {
                parent_id,
                collection,
                ordered_ids,
            } => self.reorder(*parent_id, *collection, ordered_ids),
            GraphPatch::Rename {
                target_id,
                name,
                title,
            } => self.rename(*target_id, name, title.as_deref()),
            GraphPatch::SetProperty {
                target_id,
                property,
                value,
            } => self.set_property(*target_id, property, value),
            GraphPatch::Connect { function_id, edge } => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|value| value.id == *function_id)
                    .ok_or(PatchError::TargetNotFound(*function_id))?;
                let from = function
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.from_node)
                    .ok_or(PatchError::TargetNotFound(edge.from_node))?;
                let to = function
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.to_node)
                    .ok_or(PatchError::TargetNotFound(edge.to_node))?;
                if !crate::function_nodes_can_connect(&from.kind, &to.kind) {
                    return Err(PatchError::InvalidValue(format!(
                        "节点 {} 不能连接到 {}",
                        from.name, to.name
                    )));
                }
                if function.graph.edges.iter().any(|value| value.id == edge.id) {
                    return Err(PatchError::DuplicateSymbol(edge.id));
                }
                function.graph.edges.push(edge.clone());
                Ok(())
            }
            GraphPatch::Disconnect {
                function_id,
                edge_id,
            } => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|value| value.id == *function_id)
                    .ok_or(PatchError::TargetNotFound(*function_id))?;
                remove_by_id(&mut function.graph.edges, *edge_id, |value| value.id).map(|_| ())
            }
        }
    }

    fn insert_entity(
        &mut self,
        parent_id: SymbolId,
        collection: ChildCollection,
        index: usize,
        entity: GraphEntity,
    ) -> Result<(), PatchError> {
        if self.contains_symbol(entity.id()) {
            return Err(PatchError::DuplicateSymbol(entity.id()));
        }
        match (collection, entity) {
            (ChildCollection::Menus, GraphEntity::Menu(value)) if parent_id == self.id => {
                insert_at(&mut self.menus, index, value)
            }
            (ChildCollection::Models, GraphEntity::Model(value)) if parent_id == self.id => {
                insert_at(&mut self.models, index, value)
            }
            (ChildCollection::Pages, GraphEntity::Page(value)) if parent_id == self.id => {
                insert_at(&mut self.pages, index, value)
            }
            (ChildCollection::PageEndpoints, GraphEntity::PageEndpoint(value)) => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|page| page.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut page.endpoints, index, value)
            }
            (ChildCollection::Functions, GraphEntity::Function(value)) if parent_id == self.id => {
                insert_at(&mut self.functions, index, value)
            }
            (ChildCollection::FunctionNodes, GraphEntity::FunctionNode(value)) => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|item| item.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut function.graph.nodes, index, value)
            }
            (ChildCollection::Routes, GraphEntity::Route(value)) if parent_id == self.id => {
                insert_at(&mut self.routes, index, value)
            }
            (ChildCollection::Permissions, GraphEntity::Permission(value))
                if parent_id == self.id =>
            {
                insert_at(&mut self.permissions, index, value)
            }
            (ChildCollection::MenuChildren, GraphEntity::Menu(value)) => {
                let children = find_menu_children_mut(&mut self.menus, parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(children, index, value)
            }
            (ChildCollection::Fields, GraphEntity::Field(value)) => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|item| item.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut model.fields, index, value)
            }
            (ChildCollection::ModelIndexes, GraphEntity::ModelIndex(value)) => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut model.indexes, index, value)
            }
            (ChildCollection::ModelQueries, GraphEntity::ModelQuery(value)) => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut model.queries, index, value)
            }
            (ChildCollection::ModelValidations, GraphEntity::ModelValidation(value)) => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut model.validations, index, value)
            }
            (ChildCollection::FunctionInputs, GraphEntity::Port(value)) => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|item| item.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut function.inputs, index, value)
            }
            (ChildCollection::FunctionOutputs, GraphEntity::Port(value)) => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|item| item.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut function.outputs, index, value)
            }
            _ => Err(PatchError::CollectionMismatch),
        }
    }

    fn delete_entity(&mut self, target_id: SymbolId) -> Result<(), PatchError> {
        self.take_entity(target_id).map(|_| ())
    }

    fn take_entity(&mut self, target_id: SymbolId) -> Result<GraphEntity, PatchError> {
        if let Some(value) = take_menu(&mut self.menus, target_id) {
            return Ok(GraphEntity::Menu(value));
        }
        if let Ok(value) = remove_by_id(&mut self.models, target_id, |value| value.id) {
            return Ok(GraphEntity::Model(value));
        }
        for model in &mut self.models {
            if let Ok(value) = remove_by_id(&mut model.fields, target_id, |value| value.id) {
                return Ok(GraphEntity::Field(value));
            }
            if let Ok(value) = remove_by_id(&mut model.indexes, target_id, |value| value.id) {
                return Ok(GraphEntity::ModelIndex(value));
            }
            if let Ok(value) = remove_by_id(&mut model.queries, target_id, |value| value.id) {
                return Ok(GraphEntity::ModelQuery(value));
            }
            if let Ok(value) = remove_by_id(&mut model.validations, target_id, |value| value.id) {
                return Ok(GraphEntity::ModelValidation(value));
            }
        }
        if let Ok(value) = remove_by_id(&mut self.pages, target_id, |value| value.id) {
            return Ok(GraphEntity::Page(value));
        }
        for page in &mut self.pages {
            if let Ok(value) = remove_by_id(&mut page.endpoints, target_id, |value| value.id) {
                return Ok(GraphEntity::PageEndpoint(value));
            }
        }
        if let Ok(value) = remove_by_id(&mut self.functions, target_id, |value| value.id) {
            return Ok(GraphEntity::Function(value));
        }
        for function in &mut self.functions {
            if let Ok(value) = remove_by_id(&mut function.inputs, target_id, |value| value.id) {
                return Ok(GraphEntity::Port(value));
            }
            if let Ok(value) = remove_by_id(&mut function.outputs, target_id, |value| value.id) {
                return Ok(GraphEntity::Port(value));
            }
            if let Ok(value) = remove_by_id(&mut function.graph.nodes, target_id, |value| value.id)
            {
                function
                    .graph
                    .edges
                    .retain(|edge| edge.from_node != target_id && edge.to_node != target_id);
                return Ok(GraphEntity::FunctionNode(value));
            }
        }
        if let Ok(value) = remove_by_id(&mut self.routes, target_id, |value| value.id) {
            return Ok(GraphEntity::Route(value));
        }
        if let Ok(value) = remove_by_id(&mut self.permissions, target_id, |value| value.id) {
            return Ok(GraphEntity::Permission(value));
        }
        Err(PatchError::TargetNotFound(target_id))
    }

    fn reorder(
        &mut self,
        parent_id: SymbolId,
        collection: ChildCollection,
        ordered_ids: &[SymbolId],
    ) -> Result<(), PatchError> {
        match collection {
            ChildCollection::Menus if parent_id == self.id => {
                reorder_values(&mut self.menus, ordered_ids, |value| value.id)
            }
            ChildCollection::Models if parent_id == self.id => {
                reorder_values(&mut self.models, ordered_ids, |value| value.id)
            }
            ChildCollection::Pages if parent_id == self.id => {
                reorder_values(&mut self.pages, ordered_ids, |value| value.id)
            }
            ChildCollection::PageEndpoints => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|page| page.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut page.endpoints, ordered_ids, |value| value.id)
            }
            ChildCollection::Functions if parent_id == self.id => {
                reorder_values(&mut self.functions, ordered_ids, |value| value.id)
            }
            ChildCollection::FunctionNodes => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|value| value.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut function.graph.nodes, ordered_ids, |value| value.id)
            }
            ChildCollection::Routes if parent_id == self.id => {
                reorder_values(&mut self.routes, ordered_ids, |value| value.id)
            }
            ChildCollection::Permissions if parent_id == self.id => {
                reorder_values(&mut self.permissions, ordered_ids, |value| value.id)
            }
            ChildCollection::MenuChildren => {
                let values = find_menu_children_mut(&mut self.menus, parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(values, ordered_ids, |value| value.id)
            }
            ChildCollection::Fields => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|value| value.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut model.fields, ordered_ids, |value| value.id)
            }
            ChildCollection::ModelIndexes => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut model.indexes, ordered_ids, |value| value.id)
            }
            ChildCollection::ModelQueries => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut model.queries, ordered_ids, |value| value.id)
            }
            ChildCollection::ModelValidations => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut model.validations, ordered_ids, |value| value.id)
            }
            ChildCollection::FunctionInputs => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|value| value.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut function.inputs, ordered_ids, |value| value.id)
            }
            ChildCollection::FunctionOutputs => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|value| value.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut function.outputs, ordered_ids, |value| value.id)
            }
            _ => Err(PatchError::CollectionMismatch),
        }
    }

    fn rename(
        &mut self,
        target_id: SymbolId,
        name: &str,
        title: Option<&str>,
    ) -> Result<(), PatchError> {
        if target_id == self.id {
            self.name = name.to_owned();
            if let Some(title) = title {
                self.title = title.to_owned();
            }
            return Ok(());
        }
        if name == "id"
            && self
                .models
                .iter()
                .any(|model| model.fields.iter().any(|field| field.id == target_id))
        {
            return Err(PatchError::InvalidValue(
                "id 是系统主键字段，不能作为普通字段标识".to_owned(),
            ));
        }
        if let Some((current_name, current_title)) = self.find_name_title_mut(target_id) {
            *current_name = name.to_owned();
            if let (Some(current_title), Some(title)) = (current_title, title) {
                *current_title = title.to_owned();
            }
            return Ok(());
        }
        Err(PatchError::TargetNotFound(target_id))
    }
}

include!("patch_properties.rs");

fn ensure_studio_insertable(entity: &GraphEntity) -> Result<(), PatchError> {
    let contains_reserved_id = match entity {
        GraphEntity::Model(model) => model.fields.iter().any(|field| field.name == "id"),
        GraphEntity::Field(field) => field.name == "id",
        _ => false,
    };
    if contains_reserved_id {
        return Err(PatchError::InvalidValue(
            "id 是系统主键字段，不能作为普通字段插入".to_owned(),
        ));
    }
    Ok(())
}

fn json_string(value: &Value) -> Result<String, PatchError> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| PatchError::InvalidValue("需要字符串".to_owned()))
}

fn insert_at<T>(values: &mut Vec<T>, index: usize, value: T) -> Result<(), PatchError> {
    if index > values.len() {
        return Err(PatchError::InvalidValue(format!("插入位置越界: {index}")));
    }
    values.insert(index, value);
    Ok(())
}

fn remove_by_id<T>(
    values: &mut Vec<T>,
    id: SymbolId,
    identify: impl Fn(&T) -> SymbolId,
) -> Result<T, PatchError> {
    let index = values
        .iter()
        .position(|value| identify(value) == id)
        .ok_or(PatchError::TargetNotFound(id))?;
    Ok(values.remove(index))
}

fn reorder_values<T>(
    values: &mut Vec<T>,
    ordered_ids: &[SymbolId],
    identify: impl Fn(&T) -> SymbolId,
) -> Result<(), PatchError> {
    if values.len() != ordered_ids.len()
        || !values
            .iter()
            .all(|value| ordered_ids.contains(&identify(value)))
    {
        return Err(PatchError::InvalidOrder);
    }
    let mut reordered = Vec::with_capacity(values.len());
    for id in ordered_ids {
        let index = values
            .iter()
            .position(|value| identify(value) == *id)
            .ok_or(PatchError::InvalidOrder)?;
        reordered.push(values.remove(index));
    }
    *values = reordered;
    Ok(())
}

fn menu_contains(menu: &MenuDefinition, target: SymbolId) -> bool {
    menu.id == target
        || menu
            .children
            .iter()
            .any(|value| menu_contains(value, target))
}

fn find_menu_mut(values: &mut [MenuDefinition], target: SymbolId) -> Option<&mut MenuDefinition> {
    for value in values {
        if value.id == target {
            return Some(value);
        }
        if let Some(found) = find_menu_mut(&mut value.children, target) {
            return Some(found);
        }
    }
    None
}

fn find_menu_children_mut(
    menus: &mut [MenuDefinition],
    target: SymbolId,
) -> Option<&mut Vec<MenuDefinition>> {
    find_menu_mut(menus, target).map(|menu| &mut menu.children)
}

fn take_menu(menus: &mut Vec<MenuDefinition>, target: SymbolId) -> Option<MenuDefinition> {
    take_menu_from(menus, target)
}

fn take_menu_from(values: &mut Vec<MenuDefinition>, target: SymbolId) -> Option<MenuDefinition> {
    if let Some(index) = values.iter().position(|value| value.id == target) {
        return Some(values.remove(index));
    }
    for value in values {
        if let Some(found) = take_menu_from(&mut value.children, target) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
