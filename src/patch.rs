use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ComponentNode, DataSourceDefinition, FieldDefinition, FunctionDefinition, FunctionNode,
    GraphEdge, MenuDefinition, ModelDefinition, ModelIndexDefinition, PageDefinition,
    PageStateDefinition, PermissionDefinition, PortDefinition, ProgramDefinition, RouteDefinition,
    SymbolId,
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
        entity: GraphEntity,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildCollection {
    Menus,
    MenuChildren,
    Models,
    Fields,
    ModelIndexes,
    Pages,
    ComponentChildren,
    PageStates,
    DataSources,
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
    Page(PageDefinition),
    Component(ComponentNode),
    PageState(PageStateDefinition),
    DataSource(DataSourceDefinition),
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
            Self::Page(value) => value.id,
            Self::Component(value) => value.id,
            Self::PageState(value) => value.id,
            Self::DataSource(value) => value.id,
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
    RoutePath,
    Icon,
    MenuPage,
    MenuEnabled,
    MenuPermissions,
    DefinitionState,
    ComponentProperty(String),
    ComponentContent,
    ComponentEvent(String),
    ComponentStyle,
    FieldRequired,
    FunctionNodePosition,
    PageStateInitialValue,
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
            } => self.insert_entity(*parent_id, *collection, *index, entity.clone()),
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
            (ChildCollection::ComponentChildren, GraphEntity::Component(value)) => {
                let children = find_component_children_mut(&mut self.pages, parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(children, index, value)
            }
            (ChildCollection::PageStates, GraphEntity::PageState(value)) => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|item| item.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut page.page_state, index, value)
            }
            (ChildCollection::DataSources, GraphEntity::DataSource(value)) => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|item| item.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                insert_at(&mut page.data_sources, index, value)
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
        }
        if let Ok(value) = remove_by_id(&mut self.pages, target_id, |value| value.id) {
            return Ok(GraphEntity::Page(value));
        }
        if let Some(value) = take_component(&mut self.pages, target_id) {
            return Ok(GraphEntity::Component(value));
        }
        for page in &mut self.pages {
            if let Ok(value) = remove_by_id(&mut page.page_state, target_id, |value| value.id) {
                return Ok(GraphEntity::PageState(value));
            }
            if let Ok(value) = remove_by_id(&mut page.data_sources, target_id, |value| value.id) {
                return Ok(GraphEntity::DataSource(value));
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
            ChildCollection::ComponentChildren => {
                let values = find_component_children_mut(&mut self.pages, parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(values, ordered_ids, |value| value.id)
            }
            ChildCollection::PageStates => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|value| value.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut page.page_state, ordered_ids, |value| value.id)
            }
            ChildCollection::DataSources => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|value| value.id == parent_id)
                    .ok_or(PatchError::ParentNotFound(parent_id))?;
                reorder_values(&mut page.data_sources, ordered_ids, |value| value.id)
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
        if let Some((current_name, current_title)) = self.find_name_title_mut(target_id) {
            *current_name = name.to_owned();
            if let (Some(current_title), Some(title)) = (current_title, title) {
                *current_title = title.to_owned();
            }
            return Ok(());
        }
        Err(PatchError::TargetNotFound(target_id))
    }

    fn set_property(
        &mut self,
        target_id: SymbolId,
        property: &EditableProperty,
        value: &Value,
    ) -> Result<(), PatchError> {
        match property {
            EditableProperty::RoutePath => {
                let route = self
                    .routes
                    .iter_mut()
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                route.path = json_string(value)?;
                Ok(())
            }
            EditableProperty::Icon => {
                let menu = find_menu_mut(&mut self.menus, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                menu.icon = if value.is_null() {
                    None
                } else {
                    Some(json_string(value)?)
                };
                Ok(())
            }
            EditableProperty::MenuPage => {
                let menu = find_menu_mut(&mut self.menus, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                menu.page_id = if value.is_null() {
                    None
                } else {
                    Some(json_string(value).and_then(|text| {
                        SymbolId::parse(&text)
                            .map_err(|error| PatchError::InvalidValue(error.to_string()))
                    })?)
                };
                Ok(())
            }
            EditableProperty::MenuEnabled => {
                let menu = find_menu_mut(&mut self.menus, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                menu.enabled = value.as_bool().ok_or_else(|| {
                    PatchError::InvalidValue("menu_enabled 必须是布尔值".to_owned())
                })?;
                Ok(())
            }
            EditableProperty::MenuPermissions => {
                let menu = find_menu_mut(&mut self.menus, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                menu.required_permissions = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::ComponentProperty(name) => {
                let component = find_component_mut(&mut self.pages, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                component.properties.insert(
                    name.clone(),
                    serde_json::from_value(value.clone())
                        .map_err(|error| PatchError::InvalidValue(error.to_string()))?,
                );
                Ok(())
            }
            EditableProperty::ComponentContent => {
                let component = find_component_mut(&mut self.pages, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                component.content = if value.is_null() {
                    None
                } else {
                    Some(
                        serde_json::from_value(value.clone())
                            .map_err(|error| PatchError::InvalidValue(error.to_string()))?,
                    )
                };
                Ok(())
            }
            EditableProperty::ComponentEvent(name) => {
                let component = find_component_mut(&mut self.pages, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                if value.is_null() {
                    component.events.remove(name);
                } else {
                    let id = json_string(value).and_then(|text| {
                        SymbolId::parse(&text)
                            .map_err(|error| PatchError::InvalidValue(error.to_string()))
                    })?;
                    component.events.insert(name.clone(), id);
                }
                Ok(())
            }
            EditableProperty::ComponentStyle => {
                let component = find_component_mut(&mut self.pages, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                component.style = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::FieldRequired => {
                let field = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.fields)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                field.required = value.as_bool().ok_or_else(|| {
                    PatchError::InvalidValue("field_required 必须是布尔值".to_owned())
                })?;
                Ok(())
            }
            EditableProperty::FunctionNodePosition => {
                let node = self
                    .functions
                    .iter_mut()
                    .flat_map(|function| &mut function.graph.nodes)
                    .find(|node| node.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                node.editor = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::PageStateInitialValue => {
                let state = self
                    .pages
                    .iter_mut()
                    .flat_map(|page| &mut page.page_state)
                    .find(|state| state.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                state.initial_value = value.clone();
                Ok(())
            }
            EditableProperty::DefinitionState => {
                let state = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                self.set_definition_state(target_id, state)
            }
            EditableProperty::Title => {
                let title = json_string(value)?;
                if target_id == self.id {
                    self.title = title;
                    return Ok(());
                }
                if let Some((_, Some(current_title))) = self.find_name_title_mut(target_id) {
                    *current_title = title;
                    return Ok(());
                }
                Err(PatchError::TargetNotFound(target_id))
            }
        }
    }

    fn contains_symbol(&self, target: SymbolId) -> bool {
        if self.id == target
            || self.menus.iter().any(|value| menu_contains(value, target))
            || self.models.iter().any(|value| value.id == target)
            || self.pages.iter().any(|value| value.id == target)
            || self.functions.iter().any(|value| value.id == target)
            || self.routes.iter().any(|value| value.id == target)
            || self.permissions.iter().any(|value| value.id == target)
        {
            return true;
        }
        self.models
            .iter()
            .flat_map(|model| &model.fields)
            .any(|field| field.id == target)
            || self
                .models
                .iter()
                .flat_map(|model| &model.indexes)
                .any(|index| index.id == target)
            || self.pages.iter().any(|page| {
                component_contains(&page.root, target)
                    || page.page_state.iter().any(|state| state.id == target)
                    || page.data_sources.iter().any(|source| source.id == target)
            })
            || self.functions.iter().any(|function| {
                function.inputs.iter().any(|port| port.id == target)
                    || function.outputs.iter().any(|port| port.id == target)
                    || function.graph.nodes.iter().any(|node| node.id == target)
                    || function.graph.edges.iter().any(|edge| edge.id == target)
            })
    }

    fn find_name_title_mut(
        &mut self,
        target: SymbolId,
    ) -> Option<(&mut String, Option<&mut String>)> {
        if let Some(value) = find_menu_mut(&mut self.menus, target) {
            return Some((&mut value.name, Some(&mut value.title)));
        }
        for model in &mut self.models {
            if model.id == target {
                return Some((&mut model.name, Some(&mut model.title)));
            }
            for field in &mut model.fields {
                if field.id == target {
                    return Some((&mut field.name, Some(&mut field.title)));
                }
            }
        }
        for page in &mut self.pages {
            if page.id == target {
                return Some((&mut page.name, Some(&mut page.title)));
            }
            if let Some(state) = page.page_state.iter_mut().find(|state| state.id == target) {
                return Some((&mut state.name, None));
            }
            if let Some(source) = page
                .data_sources
                .iter_mut()
                .find(|source| source.id == target)
            {
                return Some((&mut source.name, None));
            }
        }
        for function in &mut self.functions {
            if function.id == target {
                return Some((&mut function.name, Some(&mut function.title)));
            }
            if let Some(port) = function
                .inputs
                .iter_mut()
                .chain(&mut function.outputs)
                .find(|port| port.id == target)
            {
                return Some((&mut port.name, None));
            }
            if let Some(node) = function
                .graph
                .nodes
                .iter_mut()
                .find(|node| node.id == target)
            {
                return Some((&mut node.name, None));
            }
        }
        if let Some(value) = self.routes.iter_mut().find(|value| value.id == target) {
            return Some((&mut value.name, None));
        }
        if let Some(value) = self.permissions.iter_mut().find(|value| value.id == target) {
            return Some((&mut value.name, Some(&mut value.title)));
        }
        None
    }

    fn set_definition_state(
        &mut self,
        target: SymbolId,
        state: crate::DefinitionState,
    ) -> Result<(), PatchError> {
        if let Some(menu) = find_menu_mut(&mut self.menus, target) {
            menu.state = state;
            return Ok(());
        }
        for model in &mut self.models {
            if model.id == target {
                model.state = state;
                return Ok(());
            }
            if let Some(field) = model.fields.iter_mut().find(|value| value.id == target) {
                field.state = state;
                return Ok(());
            }
        }
        if let Some(page) = self.pages.iter_mut().find(|value| value.id == target) {
            page.state = state;
            return Ok(());
        }
        for page in &mut self.pages {
            if let Some(component) = find_component_node_mut(&mut page.root, target) {
                component.state = state;
                return Ok(());
            }
        }
        if let Some(function) = self.functions.iter_mut().find(|value| value.id == target) {
            function.state = state;
            return Ok(());
        }
        for function in &mut self.functions {
            if let Some(node) = function
                .graph
                .nodes
                .iter_mut()
                .find(|value| value.id == target)
            {
                node.state = state;
                return Ok(());
            }
        }
        if let Some(route) = self.routes.iter_mut().find(|value| value.id == target) {
            route.state = state;
            return Ok(());
        }
        Err(PatchError::TargetNotFound(target))
    }
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

fn component_contains(component: &ComponentNode, target: SymbolId) -> bool {
    component.id == target
        || component
            .children
            .iter()
            .any(|value| component_contains(value, target))
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

fn find_component_mut(
    pages: &mut [PageDefinition],
    target: SymbolId,
) -> Option<&mut ComponentNode> {
    for page in pages {
        if let Some(value) = find_component_node_mut(&mut page.root, target) {
            return Some(value);
        }
    }
    None
}

fn find_component_node_mut(
    value: &mut ComponentNode,
    target: SymbolId,
) -> Option<&mut ComponentNode> {
    if value.id == target {
        return Some(value);
    }
    for child in &mut value.children {
        if let Some(found) = find_component_node_mut(child, target) {
            return Some(found);
        }
    }
    None
}

fn find_component_children_mut(
    pages: &mut [PageDefinition],
    target: SymbolId,
) -> Option<&mut Vec<ComponentNode>> {
    find_component_mut(pages, target).map(|value| &mut value.children)
}

fn take_component(pages: &mut [PageDefinition], target: SymbolId) -> Option<ComponentNode> {
    for page in pages {
        if let Some(value) = take_component_from(&mut page.root.children, target) {
            return Some(value);
        }
    }
    None
}

fn take_component_from(values: &mut Vec<ComponentNode>, target: SymbolId) -> Option<ComponentNode> {
    if let Some(index) = values.iter().position(|value| value.id == target) {
        return Some(values.remove(index));
    }
    for value in values {
        if let Some(found) = take_component_from(&mut value.children, target) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::{
        ComponentStyle, DefinitionState, FunctionGraph, FunctionNodeEditor, FunctionNodeKind,
        ValueType,
    };

    fn editable_program() -> (ProgramDefinition, SymbolId, SymbolId, SymbolId) {
        let mut program = ProgramDefinition::empty("studio-test", "Studio 测试");
        let model_id = SymbolId::new();
        let page_id = SymbolId::new();
        let function_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            fields: Vec::new(),
            indexes: Vec::new(),
        });
        program.pages.push(PageDefinition {
            id: page_id,
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            root: ComponentNode {
                id: SymbolId::new(),
                component: "layout.section".to_owned(),
                state: DefinitionState::Known,
                properties: BTreeMap::new(),
                content: None,
                events: BTreeMap::new(),
                children: Vec::new(),
                style: ComponentStyle::default(),
            },
            page_state: Vec::new(),
            data_sources: Vec::new(),
        });
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "load-assets".to_owned(),
            title: "加载资产".to_owned(),
            state: DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph::default(),
            required_permissions: Vec::new(),
        });
        (program, model_id, page_id, function_id)
    }

    #[test]
    fn studio_contract_entities_share_one_atomic_patch_protocol() -> anyhow::Result<()> {
        let (mut program, model_id, page_id, function_id) = editable_program();
        let field_id = SymbolId::new();
        let state_id = SymbolId::new();
        let source_id = SymbolId::new();
        let port_id = SymbolId::new();
        let node_id = SymbolId::new();
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::Insert {
                    parent_id: model_id,
                    collection: ChildCollection::Fields,
                    index: 0,
                    entity: GraphEntity::Field(FieldDefinition {
                        id: field_id,
                        name: "name".to_owned(),
                        title: "名称".to_owned(),
                        value_type: ValueType::Text,
                        state: DefinitionState::Known,
                        required: true,
                        relation_model_id: None,
                    }),
                },
                GraphPatch::Insert {
                    parent_id: page_id,
                    collection: ChildCollection::PageStates,
                    index: 0,
                    entity: GraphEntity::PageState(PageStateDefinition {
                        id: state_id,
                        name: "keyword".to_owned(),
                        value_type: ValueType::Text,
                        initial_value: json!(""),
                    }),
                },
                GraphPatch::Insert {
                    parent_id: page_id,
                    collection: ChildCollection::DataSources,
                    index: 0,
                    entity: GraphEntity::DataSource(DataSourceDefinition {
                        id: source_id,
                        name: "assets".to_owned(),
                        function_id,
                        parameters: BTreeMap::new(),
                    }),
                },
                GraphPatch::Insert {
                    parent_id: function_id,
                    collection: ChildCollection::FunctionInputs,
                    index: 0,
                    entity: GraphEntity::Port(PortDefinition {
                        id: port_id,
                        name: "keyword".to_owned(),
                        value_type: ValueType::Text,
                    }),
                },
                GraphPatch::Insert {
                    parent_id: function_id,
                    collection: ChildCollection::FunctionNodes,
                    index: 0,
                    entity: GraphEntity::FunctionNode(FunctionNode {
                        id: node_id,
                        name: "input".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Input { port_id },
                    }),
                },
            ],
        })?;

        program.apply_patch(&GraphPatch::Rename {
            target_id: state_id,
            name: "search".to_owned(),
            title: None,
        })?;
        program.apply_patch(&GraphPatch::SetProperty {
            target_id: state_id,
            property: EditableProperty::PageStateInitialValue,
            value: json!("AIO"),
        })?;
        program.apply_patch(&GraphPatch::SetProperty {
            target_id: node_id,
            property: EditableProperty::FunctionNodePosition,
            value: json!({"x": 120, "y": 80}),
        })?;

        assert_eq!(program.models[0].fields[0].id, field_id);
        assert_eq!(program.pages[0].page_state[0].name, "search");
        assert_eq!(program.pages[0].page_state[0].initial_value, json!("AIO"));
        assert_eq!(program.pages[0].data_sources[0].id, source_id);
        assert_eq!(program.functions[0].inputs[0].id, port_id);
        assert_eq!(program.functions[0].graph.nodes[0].editor.x, 120);
        Ok(())
    }

    #[test]
    fn root_menu_is_scene_without_an_extra_definition_layer() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("menu-test", "菜单测试");
        let scene_id = SymbolId::new();
        let child_id = SymbolId::new();
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::Insert {
                    parent_id: program.id,
                    collection: ChildCollection::Menus,
                    index: 0,
                    entity: GraphEntity::Menu(MenuDefinition {
                        id: scene_id,
                        name: "operations".to_owned(),
                        title: "运营场景".to_owned(),
                        state: DefinitionState::Known,
                        icon: None,
                        page_id: None,
                        enabled: true,
                        children: Vec::new(),
                        required_permissions: Vec::new(),
                    }),
                },
                GraphPatch::Insert {
                    parent_id: scene_id,
                    collection: ChildCollection::MenuChildren,
                    index: 0,
                    entity: GraphEntity::Menu(MenuDefinition {
                        id: child_id,
                        name: "dashboard".to_owned(),
                        title: "运营看板".to_owned(),
                        state: DefinitionState::Known,
                        icon: None,
                        page_id: None,
                        enabled: true,
                        children: Vec::new(),
                        required_permissions: Vec::new(),
                    }),
                },
            ],
        })?;

        let page_id = SymbolId::new();
        let permission_id = SymbolId::new();
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 1,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::SetProperty {
                    target_id: child_id,
                    property: EditableProperty::MenuEnabled,
                    value: json!(false),
                },
                GraphPatch::SetProperty {
                    target_id: child_id,
                    property: EditableProperty::MenuPage,
                    value: json!(page_id.to_string()),
                },
                GraphPatch::SetProperty {
                    target_id: child_id,
                    property: EditableProperty::MenuPermissions,
                    value: json!([permission_id]),
                },
            ],
        })?;

        assert_eq!(program.menus[0].id, scene_id);
        assert_eq!(program.menus[0].children[0].id, child_id);
        assert!(!program.menus[0].children[0].enabled);
        assert_eq!(program.menus[0].children[0].page_id, Some(page_id));
        assert_eq!(
            program.menus[0].children[0].required_permissions,
            vec![permission_id]
        );
        Ok(())
    }

    #[test]
    fn failed_batch_does_not_partially_mutate_definition() {
        let (mut program, _, page_id, _) = editable_program();
        let before = program.clone();
        let duplicate_id = program.pages[0].id;
        let result = program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::Rename {
                    target_id: page_id,
                    name: "changed".to_owned(),
                    title: Some("已修改".to_owned()),
                },
                GraphPatch::Insert {
                    parent_id: page_id,
                    collection: ChildCollection::PageStates,
                    index: 0,
                    entity: GraphEntity::PageState(PageStateDefinition {
                        id: duplicate_id,
                        name: "duplicate".to_owned(),
                        value_type: ValueType::Text,
                        initial_value: json!(null),
                    }),
                },
            ],
        });
        assert!(matches!(result, Err(PatchError::DuplicateSymbol(_))));
        assert_eq!(program, before);
    }

    #[test]
    fn applies_one_hundred_patch_batch_within_acceptance_budget() -> anyhow::Result<()> {
        let (mut program, _, page_id, _) = editable_program();
        let patches = (0..100)
            .map(|index| GraphPatch::Insert {
                parent_id: page_id,
                collection: ChildCollection::PageStates,
                index,
                entity: GraphEntity::PageState(PageStateDefinition {
                    id: SymbolId::new(),
                    name: format!("state-{index}"),
                    value_type: ValueType::Integer,
                    initial_value: json!(index),
                }),
            })
            .collect();
        let started = Instant::now();
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            patches,
            origin: PatchOrigin::Studio,
        })?;
        assert_eq!(program.pages[0].page_state.len(), 100);
        assert!(started.elapsed() < Duration::from_secs(1));
        Ok(())
    }
}
