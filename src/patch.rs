use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FieldDefinition, FunctionDefinition, FunctionNode, GraphEdge, MenuDefinition, ModelDefinition,
    ModelIndexDefinition, PageDefinition, PermissionDefinition, PortDefinition, ProgramDefinition,
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
    MenuRowActions,
    PageRenderer,
    DefinitionState,
    FieldRequired,
    FieldValueType,
    FieldOptions,
    ModelIndexFields,
    ModelIndexPurpose,
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
            EditableProperty::MenuRowActions => {
                let menu = find_menu_mut(&mut self.menus, target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                menu.row_actions = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::PageRenderer => {
                let page = self
                    .pages
                    .iter_mut()
                    .find(|page| page.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                page.renderer = serde_json::from_value(value.clone())
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
            EditableProperty::FieldValueType => {
                let field = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.fields)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let value_type = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                field.relation_model_id = match &value_type {
                    crate::ValueType::Object { model_id } => Some(*model_id),
                    _ => None,
                };
                field.value_type = value_type;
                Ok(())
            }
            EditableProperty::FieldOptions => {
                let field = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.fields)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                field.options = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::ModelIndexFields => {
                let index = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.indexes)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let fields = serde_json::from_value::<Vec<SymbolId>>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if fields.is_empty() {
                    return Err(PatchError::InvalidValue("索引至少需要一个字段".to_owned()));
                }
                index.fields = fields;
                Ok(())
            }
            EditableProperty::ModelIndexPurpose => {
                let index = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.indexes)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                index.purpose = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
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
mod tests {
    use super::*;
    use crate::{MenuActionAccess, MenuRowActions, PageRendererDefinition, TableDefinition};

    #[test]
    fn page_renderer_and_menu_actions_use_the_same_patch_protocol() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let page_id = SymbolId::new();
        let menu_id = SymbolId::new();
        let permission_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
        });
        program.menus.push(MenuDefinition {
            id: menu_id,
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            icon: None,
            page_id: Some(page_id),
            enabled: true,
            children: Vec::new(),
            required_permissions: Vec::new(),
            row_actions: MenuRowActions::default(),
        });
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::SetProperty {
                    target_id: page_id,
                    property: EditableProperty::PageRenderer,
                    value: serde_json::to_value(PageRendererDefinition::CrudTable {
                        table: TableDefinition::default(),
                    })?,
                },
                GraphPatch::SetProperty {
                    target_id: menu_id,
                    property: EditableProperty::MenuRowActions,
                    value: serde_json::to_value(MenuRowActions {
                        detail: MenuActionAccess::Public,
                        edit: MenuActionAccess::Permission { permission_id },
                        delete: MenuActionAccess::Hidden,
                    })?,
                },
            ],
        })?;
        assert!(matches!(
            program.pages[0].renderer,
            PageRendererDefinition::CrudTable { .. }
        ));
        assert_eq!(
            program.menus[0].row_actions.edit,
            MenuActionAccess::Permission { permission_id }
        );
        Ok(())
    }

    #[test]
    fn patch_batch_is_atomic_on_invalid_target() {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let original = program.clone();
        let result = program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![GraphPatch::Rename {
                target_id: SymbolId::new(),
                name: "missing".to_owned(),
                title: None,
            }],
        });
        assert!(result.is_err());
        assert_eq!(program, original);
    }

    #[test]
    fn model_grid_properties_update_fields_and_indexes() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let model_id = SymbolId::new();
        let first_field_id = SymbolId::new();
        let second_field_id = SymbolId::new();
        let index_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            fields: vec![
                FieldDefinition {
                    id: first_field_id,
                    name: "name".to_owned(),
                    title: "名称".to_owned(),
                    value_type: crate::ValueType::Text,
                    state: crate::DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation_model_id: None,
                },
                FieldDefinition {
                    id: second_field_id,
                    name: "count".to_owned(),
                    title: "数量".to_owned(),
                    value_type: crate::ValueType::Integer,
                    state: crate::DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation_model_id: None,
                },
            ],
            indexes: vec![ModelIndexDefinition {
                id: index_id,
                fields: vec![first_field_id],
                purpose: crate::IndexPurpose::Filter,
            }],
        });

        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::SetProperty {
                    target_id: first_field_id,
                    property: EditableProperty::FieldValueType,
                    value: serde_json::to_value(crate::ValueType::Boolean)?,
                },
                GraphPatch::SetProperty {
                    target_id: first_field_id,
                    property: EditableProperty::FieldOptions,
                    value: serde_json::to_value(crate::FieldOptions {
                        list_visible: false,
                        unique: true,
                        validation: crate::FieldValidation {
                            min_length: Some(2),
                            ..crate::FieldValidation::default()
                        },
                        ..crate::FieldOptions::default()
                    })?,
                },
                GraphPatch::SetProperty {
                    target_id: index_id,
                    property: EditableProperty::ModelIndexFields,
                    value: serde_json::to_value(vec![first_field_id, second_field_id])?,
                },
                GraphPatch::SetProperty {
                    target_id: index_id,
                    property: EditableProperty::ModelIndexPurpose,
                    value: serde_json::to_value(crate::IndexPurpose::Sort)?,
                },
            ],
        })?;

        assert_eq!(
            program.models[0].fields[0].value_type,
            crate::ValueType::Boolean
        );
        assert!(!program.models[0].fields[0].options.list_visible);
        assert!(program.models[0].fields[0].options.unique);
        assert_eq!(
            program.models[0].fields[0].options.validation.min_length,
            Some(2)
        );
        assert_eq!(
            program.models[0].indexes[0].fields,
            vec![first_field_id, second_field_id]
        );
        assert_eq!(
            program.models[0].indexes[0].purpose,
            crate::IndexPurpose::Sort
        );
        Ok(())
    }
}
