use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    EndpointImplementationDefinition, FieldDefinition, FunctionDefinition, FunctionNode, GraphEdge,
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
                self.insert_entity(*parent_id, *collection, *index, entity.clone())
            }
            GraphPatch::Delete { target_id } => {
                self.ensure_studio_owned_target(*target_id)?;
                self.delete_entity(*target_id)
            }
            GraphPatch::Move {
                target_id,
                parent_id,
                collection,
                index,
            } => {
                self.ensure_studio_owned_target(*target_id)?;
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
            } => {
                self.ensure_studio_mutable_target(*target_id)?;
                if self.pages.iter().any(|page| {
                    page.id == *target_id
                        && page.name != *name
                        && page.endpoints.iter().any(endpoint_is_native)
                }) {
                    return Err(native_contract_patch_error());
                }
                self.rename(*target_id, name, title.as_deref())
            }
            GraphPatch::SetProperty {
                target_id,
                property,
                value,
            } => {
                self.ensure_studio_mutable_target(*target_id)?;
                self.set_property(*target_id, property, value)
            }
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

    fn ensure_studio_owned_target(&self, target_id: SymbolId) -> Result<(), PatchError> {
        for page in &self.pages {
            if page.id == target_id && page.endpoints.iter().any(endpoint_is_native) {
                return Err(native_contract_patch_error());
            }
            if page
                .endpoints
                .iter()
                .any(|endpoint| endpoint.id == target_id && endpoint_is_native(endpoint))
            {
                return Err(native_contract_patch_error());
            }
        }
        Ok(())
    }

    fn ensure_studio_mutable_target(&self, target_id: SymbolId) -> Result<(), PatchError> {
        if self.pages.iter().any(|page| {
            page.endpoints
                .iter()
                .any(|endpoint| endpoint.id == target_id && endpoint_is_native(endpoint))
        }) {
            return Err(native_contract_patch_error());
        }
        Ok(())
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
            EditableProperty::RoutePermissions => {
                let route = self
                    .routes
                    .iter_mut()
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                route.required_permissions = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
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
            EditableProperty::PermissionEffects => {
                let permission = self
                    .permissions
                    .iter_mut()
                    .find(|permission| permission.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                permission.allowed_effects = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::FunctionPermissions => {
                let function = self
                    .functions
                    .iter_mut()
                    .find(|function| function.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                function.required_permissions = serde_json::from_value(value.clone())
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
            EditableProperty::PageEndpoint => {
                let endpoint = self
                    .pages
                    .iter_mut()
                    .flat_map(|page| &mut page.endpoints)
                    .find(|endpoint| endpoint.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                if endpoint_is_native(endpoint) {
                    return Err(native_contract_patch_error());
                }
                let replacement = serde_json::from_value::<PageEndpointDefinition>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if replacement.id != target_id {
                    return Err(PatchError::InvalidValue(
                        "页面接口更新不能改变 SymbolId".to_owned(),
                    ));
                }
                if endpoint_is_native(&replacement) {
                    return Err(native_contract_patch_error());
                }
                *endpoint = replacement;
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
                field.value_type = value_type;
                Ok(())
            }
            EditableProperty::FieldRelation => {
                let field = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.fields)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                field.relation = serde_json::from_value(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
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
            EditableProperty::ModelIndexUnique => {
                let index = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.indexes)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                index.unique = value.as_bool().ok_or_else(|| {
                    PatchError::InvalidValue("索引唯一约束必须是布尔值".to_owned())
                })?;
                Ok(())
            }
            EditableProperty::ModelQuery => {
                let query = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.queries)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let replacement = serde_json::from_value::<ModelQueryDefinition>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if replacement.id != target_id {
                    return Err(PatchError::InvalidValue(
                        "查询更新不能改变 SymbolId".to_owned(),
                    ));
                }
                *query = replacement;
                Ok(())
            }
            EditableProperty::ModelValidation => {
                let validation = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.validations)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let replacement =
                    serde_json::from_value::<ModelValidationDefinition>(value.clone())
                        .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if replacement.id != target_id {
                    return Err(PatchError::InvalidValue(
                        "模型校验更新不能改变 SymbolId".to_owned(),
                    ));
                }
                *validation = replacement;
                Ok(())
            }
            EditableProperty::ModelPrimaryKey => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                model.primary_key =
                    serde_json::from_value::<ModelPrimaryKeyDefinition>(value.clone())
                        .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::ModelAudit => {
                let model = self
                    .models
                    .iter_mut()
                    .find(|model| model.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                model.audit = serde_json::from_value::<ModelAuditDefinition>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                Ok(())
            }
            EditableProperty::FunctionPort => {
                let port = self
                    .functions
                    .iter_mut()
                    .flat_map(|function| function.inputs.iter_mut().chain(&mut function.outputs))
                    .find(|port| port.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let replacement = serde_json::from_value::<PortDefinition>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if replacement.id != target_id {
                    return Err(PatchError::InvalidValue(
                        "函数端口更新不能改变 SymbolId".to_owned(),
                    ));
                }
                *port = replacement;
                Ok(())
            }
            EditableProperty::FunctionNode => {
                let node = self
                    .functions
                    .iter_mut()
                    .flat_map(|function| &mut function.graph.nodes)
                    .find(|node| node.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let replacement = serde_json::from_value::<FunctionNode>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if replacement.id != target_id {
                    return Err(PatchError::InvalidValue(
                        "函数节点更新不能改变 SymbolId".to_owned(),
                    ));
                }
                *node = replacement;
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
                if let Some(endpoint) = self
                    .pages
                    .iter_mut()
                    .flat_map(|page| &mut page.endpoints)
                    .find(|endpoint| endpoint.id == target_id)
                {
                    endpoint.title = title;
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
        self.pages.iter().any(|page| {
            page.endpoints.iter().any(|endpoint| {
                endpoint.id == target
                    || endpoint.inputs.iter().any(|input| input.id == target)
                    || endpoint.outputs.iter().any(|output| output.id == target)
            })
        }) || self
            .models
            .iter()
            .flat_map(|model| &model.fields)
            .any(|field| field.id == target)
            || self
                .models
                .iter()
                .flat_map(|model| &model.indexes)
                .any(|index| index.id == target)
            || self
                .models
                .iter()
                .flat_map(|model| &model.queries)
                .any(|query| query.id == target)
            || self
                .models
                .iter()
                .flat_map(|model| &model.validations)
                .any(|validation| validation.id == target)
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
        for page in &mut self.pages {
            if let Some(endpoint) = page.endpoints.iter_mut().find(|value| value.id == target) {
                endpoint.state = state;
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
    let contains_native_contract = match entity {
        GraphEntity::Page(page) => page.endpoints.iter().any(endpoint_is_native),
        GraphEntity::PageEndpoint(endpoint) => endpoint_is_native(endpoint),
        _ => false,
    };
    if contains_native_contract {
        return Err(native_contract_patch_error());
    }
    Ok(())
}

fn endpoint_is_native(endpoint: &PageEndpointDefinition) -> bool {
    matches!(
        endpoint.implementation,
        EndpointImplementationDefinition::Native { .. }
    )
}

fn native_contract_patch_error() -> PatchError {
    PatchError::InvalidValue("原生接口元数据由插件声明维护，只能编辑约定契约".to_owned())
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
    use crate::{
        EffectKind, FunctionGraph, MenuActionAccess, MenuRowActions, PageEndpointDefinition,
        PageRendererDefinition, PermissionDefinition, RestMethod, TableDefinition,
    };

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
            endpoints: Vec::new(),
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
    fn permission_effects_use_property_patch() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let permission_id = SymbolId::new();
        program.permissions.push(PermissionDefinition {
            id: permission_id,
            name: "asset:write".to_owned(),
            title: "维护资产".to_owned(),
            allowed_effects: Vec::new(),
        });
        program.apply_patch(&GraphPatch::SetProperty {
            target_id: permission_id,
            property: EditableProperty::PermissionEffects,
            value: serde_json::json!([EffectKind::DatabaseRead, EffectKind::DatabaseWrite]),
        })?;
        assert_eq!(
            program.permissions[0].allowed_effects,
            vec![EffectKind::DatabaseRead, EffectKind::DatabaseWrite]
        );
        Ok(())
    }

    #[test]
    fn route_permissions_use_property_patch() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let page_id = SymbolId::new();
        let route_id = SymbolId::new();
        let permission_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: Vec::new(),
        });
        program.routes.push(RouteDefinition {
            id: route_id,
            name: "assets".to_owned(),
            path: "/assets".to_owned(),
            page_id,
            state: crate::DefinitionState::Known,
            required_permissions: Vec::new(),
        });

        program.apply_patch(&GraphPatch::SetProperty {
            target_id: route_id,
            property: EditableProperty::RoutePermissions,
            value: serde_json::json!([permission_id]),
        })?;

        assert_eq!(program.routes[0].required_permissions, vec![permission_id]);
        Ok(())
    }

    #[test]
    fn function_permissions_use_property_patch() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let function_id = SymbolId::new();
        let permission_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "load_asset".to_owned(),
            title: "读取资产".to_owned(),
            state: crate::DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph::default(),
            required_permissions: Vec::new(),
        });

        program.apply_patch(&GraphPatch::SetProperty {
            target_id: function_id,
            property: EditableProperty::FunctionPermissions,
            value: serde_json::json!([permission_id]),
        })?;

        assert_eq!(
            program.functions[0].required_permissions,
            vec![permission_id]
        );
        Ok(())
    }

    #[test]
    fn function_ports_and_nodes_use_replacement_properties() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let function_id = SymbolId::new();
        let port_id = SymbolId::new();
        let node_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "load_asset".to_owned(),
            title: "读取资产".to_owned(),
            state: crate::DefinitionState::Known,
            inputs: vec![PortDefinition {
                id: port_id,
                name: "asset_id".to_owned(),
                value_type: crate::ValueType::Text,
            }],
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![FunctionNode {
                    id: node_id,
                    name: "asset_id".to_owned(),
                    state: crate::DefinitionState::Known,
                    editor: crate::FunctionNodeEditor::default(),
                    kind: crate::FunctionNodeKind::Input { port_id },
                }],
                edges: Vec::new(),
            },
            required_permissions: Vec::new(),
        });
        let updated_port = PortDefinition {
            id: port_id,
            name: "id".to_owned(),
            value_type: crate::ValueType::Integer,
        };
        let updated_node = FunctionNode {
            id: node_id,
            name: "return".to_owned(),
            state: crate::DefinitionState::Known,
            editor: crate::FunctionNodeEditor { x: 120, y: 80 },
            kind: crate::FunctionNodeKind::Return,
        };

        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::SetProperty {
                    target_id: port_id,
                    property: EditableProperty::FunctionPort,
                    value: serde_json::to_value(updated_port.clone())?,
                },
                GraphPatch::SetProperty {
                    target_id: node_id,
                    property: EditableProperty::FunctionNode,
                    value: serde_json::to_value(updated_node.clone())?,
                },
            ],
        })?;

        assert_eq!(program.functions[0].inputs, vec![updated_port]);
        assert_eq!(program.functions[0].graph.nodes, vec![updated_node]);
        Ok(())
    }

    #[test]
    fn function_graph_position_and_edges_use_graph_patches() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let function_id = SymbolId::new();
        let source_node_id = SymbolId::new();
        let target_node_id = SymbolId::new();
        let edge_id = SymbolId::new();
        let node = |id, name: &str, kind| FunctionNode {
            id,
            name: name.to_owned(),
            state: crate::DefinitionState::Known,
            editor: crate::FunctionNodeEditor::default(),
            kind,
        };
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "load_asset".to_owned(),
            title: "读取资产".to_owned(),
            state: crate::DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![
                    node(
                        source_node_id,
                        "source",
                        crate::FunctionNodeKind::Constant {
                            value: serde_json::json!("asset"),
                            value_type: crate::ValueType::Text,
                        },
                    ),
                    node(target_node_id, "target", crate::FunctionNodeKind::Return),
                ],
                edges: Vec::new(),
            },
            required_permissions: Vec::new(),
        });
        let position = crate::FunctionNodeEditor { x: 320, y: 180 };
        let edge = GraphEdge {
            id: edge_id,
            from_node: source_node_id,
            from_port: "out".to_owned(),
            to_node: target_node_id,
            to_port: "in".to_owned(),
        };

        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::SetProperty {
                    target_id: source_node_id,
                    property: EditableProperty::FunctionNodePosition,
                    value: serde_json::to_value(position)?,
                },
                GraphPatch::Connect {
                    function_id,
                    edge: edge.clone(),
                },
            ],
        })?;

        assert_eq!(program.functions[0].graph.nodes[0].editor, position);
        assert_eq!(program.functions[0].graph.edges, vec![edge]);

        program.apply_patch(&GraphPatch::Disconnect {
            function_id,
            edge_id,
        })?;
        assert!(program.functions[0].graph.edges.is_empty());
        Ok(())
    }

    #[test]
    fn function_graph_rejects_edges_into_constant_nodes() {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let function_id = SymbolId::new();
        let boolean_id = SymbolId::new();
        let constant_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "check_asset".to_owned(),
            title: "检查资产".to_owned(),
            state: crate::DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![
                    FunctionNode {
                        id: boolean_id,
                        name: "boolean".to_owned(),
                        state: crate::DefinitionState::Known,
                        editor: crate::FunctionNodeEditor::default(),
                        kind: crate::FunctionNodeKind::Boolean {
                            operator: crate::BooleanOperator::And,
                        },
                    },
                    FunctionNode {
                        id: constant_id,
                        name: "constant".to_owned(),
                        state: crate::DefinitionState::Known,
                        editor: crate::FunctionNodeEditor::default(),
                        kind: crate::FunctionNodeKind::Constant {
                            value: serde_json::json!("value"),
                            value_type: crate::ValueType::Text,
                        },
                    },
                ],
                edges: Vec::new(),
            },
            required_permissions: Vec::new(),
        });

        let error = program
            .apply_patch(&GraphPatch::Connect {
                function_id,
                edge: GraphEdge {
                    id: SymbolId::new(),
                    from_node: boolean_id,
                    from_port: "out".to_owned(),
                    to_node: constant_id,
                    to_port: "in".to_owned(),
                },
            })
            .err();

        assert!(error.is_some_and(|error| error.to_string().contains("不能连接")));
        assert!(program.functions[0].graph.edges.is_empty());
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
    fn page_endpoint_uses_page_owned_patch_collection() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let page_id = SymbolId::new();
        let endpoint_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: Vec::new(),
        });
        let endpoint = PageEndpointDefinition {
            id: endpoint_id,
            title: "归档资产".to_owned(),
            description: "归档指定资产".to_owned(),
            state: crate::DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Convention,
            method: RestMethod::Post,
            path: "/api/assets/archive".to_owned(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        program.apply_patch(&GraphPatch::Insert {
            parent_id: page_id,
            collection: ChildCollection::PageEndpoints,
            index: 0,
            entity: GraphEntity::PageEndpoint(endpoint.clone()),
        })?;
        assert_eq!(program.pages[0].endpoints, vec![endpoint.clone()]);

        let mut updated = endpoint;
        updated.method = RestMethod::Delete;
        program.apply_patch(&GraphPatch::SetProperty {
            target_id: endpoint_id,
            property: EditableProperty::PageEndpoint,
            value: serde_json::to_value(updated.clone())?,
        })?;
        assert_eq!(program.pages[0].endpoints, vec![updated]);
        Ok(())
    }

    #[test]
    fn native_endpoint_and_owning_page_are_read_only_for_graph_patch() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let page_id = SymbolId::new();
        let endpoint_id = SymbolId::new();
        let endpoint = PageEndpointDefinition {
            id: endpoint_id,
            title: "资产列表".to_owned(),
            description: "由资产插件提供".to_owned(),
            state: crate::DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Native {
                plugin_id: "asset-hub".to_owned(),
            },
            method: RestMethod::Get,
            path: "/api/asset-hub/assets".to_owned(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        program.pages.push(PageDefinition {
            id: page_id,
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: vec![endpoint.clone()],
        });
        let original = program.clone();

        let endpoint_error = program
            .apply_patch(&GraphPatch::Delete {
                target_id: endpoint_id,
            })
            .err()
            .ok_or_else(|| anyhow::anyhow!("删除原生接口必须失败"))?;
        assert!(endpoint_error.to_string().contains("插件声明维护"));
        assert_eq!(program, original);

        let page_error = program
            .apply_patch(&GraphPatch::Delete { target_id: page_id })
            .err()
            .ok_or_else(|| anyhow::anyhow!("删除原生接口页面必须失败"))?;
        assert!(page_error.to_string().contains("插件声明维护"));
        assert_eq!(program, original);

        let insert_error = program
            .apply_patch(&GraphPatch::Insert {
                parent_id: page_id,
                collection: ChildCollection::PageEndpoints,
                index: 1,
                entity: GraphEntity::PageEndpoint(endpoint),
            })
            .err()
            .ok_or_else(|| anyhow::anyhow!("GraphPatch 不能伪造原生接口"))?;
        assert!(insert_error.to_string().contains("只能编辑约定契约"));
        assert_eq!(program, original);
        Ok(())
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
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: first_field_id,
                    name: "name".to_owned(),
                    title: "名称".to_owned(),
                    value_type: crate::ValueType::Text,
                    state: crate::DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: second_field_id,
                    name: "count".to_owned(),
                    title: "数量".to_owned(),
                    value_type: crate::ValueType::Integer,
                    state: crate::DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: vec![ModelIndexDefinition {
                id: index_id,
                fields: vec![first_field_id],
                unique: false,
            }],
            queries: Vec::new(),
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
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
                    property: EditableProperty::ModelIndexUnique,
                    value: serde_json::json!(true),
                },
                GraphPatch::SetProperty {
                    target_id: model_id,
                    property: EditableProperty::ModelPrimaryKey,
                    value: serde_json::json!(crate::ModelPrimaryKeyDefinition {
                        generation: crate::PrimaryKeyGeneration::AutoIncrement,
                    }),
                },
                GraphPatch::SetProperty {
                    target_id: model_id,
                    property: EditableProperty::ModelAudit,
                    value: serde_json::json!(crate::ModelAuditDefinition {
                        fields: vec![crate::ModelAuditField {
                            kind: crate::AuditFieldKind::Version,
                            field_id: second_field_id,
                        }],
                    }),
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
        assert_eq!(program.models[0].indexes[0].unique, true);
        assert_eq!(
            program.models[0].primary_key.generation,
            crate::PrimaryKeyGeneration::AutoIncrement
        );
        assert_eq!(
            program.models[0].audit.fields[0].kind,
            crate::AuditFieldKind::Version
        );
        Ok(())
    }

    #[test]
    fn system_id_cannot_be_inserted_as_a_regular_field() {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let model_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: Vec::new(),
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
        });

        let error = program
            .apply_patch(&GraphPatch::Insert {
                parent_id: model_id,
                collection: ChildCollection::Fields,
                index: 0,
                entity: GraphEntity::Field(FieldDefinition {
                    id: SymbolId::new(),
                    name: "id".to_owned(),
                    title: "ID".to_owned(),
                    value_type: crate::ValueType::Text,
                    state: crate::DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: None,
                }),
            })
            .expect_err("系统 id 不能作为普通字段插入");
        assert!(error.to_string().contains("系统主键字段"));
    }

    #[test]
    fn model_designer_updates_structured_query_validation_and_relation() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let model_id = SymbolId::new();
        let source_field_id = SymbolId::new();
        let target_field_id = SymbolId::new();
        let query_id = SymbolId::new();
        let validation_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: crate::DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: source_field_id,
                    name: "owner".to_owned(),
                    title: "负责人".to_owned(),
                    value_type: crate::ValueType::Text,
                    state: crate::DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: target_field_id,
                    name: "owner_name".to_owned(),
                    title: "负责人名称".to_owned(),
                    value_type: crate::ValueType::Text,
                    state: crate::DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: vec![crate::ModelQueryDefinition {
                id: query_id,
                name: "by_owner".to_owned(),
                title: "按负责人".to_owned(),
                conjunction: crate::QueryConjunction::All,
                conditions: Vec::new(),
            }],
            validations: vec![crate::ModelValidationDefinition {
                id: validation_id,
                message: "负责人不能为空".to_owned(),
                rule: crate::ModelValidationRule::RequiredWhenPresent {
                    field_id: source_field_id,
                    when_field_id: target_field_id,
                },
            }],
            audit: crate::ModelAuditDefinition::default(),
        });

        let relation = crate::FieldRelation {
            kind: crate::RelationKind::ManyToOne,
            target_model_id: model_id,
            target_field_id,
        };
        let updated_query = crate::ModelQueryDefinition {
            id: query_id,
            name: "by_owner_name".to_owned(),
            title: "按负责人名称".to_owned(),
            conjunction: crate::QueryConjunction::Any,
            conditions: vec![crate::QueryCondition::Field {
                field_id: target_field_id,
                operator: crate::QueryOperator::Contains,
                parameter: "owner_name".to_owned(),
            }],
        };
        let updated_validation = crate::ModelValidationDefinition {
            id: validation_id,
            message: "负责人字段必须一起填写".to_owned(),
            rule: crate::ModelValidationRule::FieldsRequiredTogether {
                field_ids: vec![source_field_id, target_field_id],
            },
        };
        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::SetProperty {
                    target_id: source_field_id,
                    property: EditableProperty::FieldRelation,
                    value: serde_json::to_value(relation.clone())?,
                },
                GraphPatch::SetProperty {
                    target_id: source_field_id,
                    property: EditableProperty::FieldValueType,
                    value: serde_json::to_value(crate::ValueType::Object { model_id })?,
                },
                GraphPatch::SetProperty {
                    target_id: query_id,
                    property: EditableProperty::ModelQuery,
                    value: serde_json::to_value(updated_query.clone())?,
                },
                GraphPatch::SetProperty {
                    target_id: validation_id,
                    property: EditableProperty::ModelValidation,
                    value: serde_json::to_value(updated_validation.clone())?,
                },
            ],
        })?;

        assert_eq!(program.models[0].fields[0].relation, Some(relation));
        assert_eq!(program.models[0].queries, vec![updated_query]);
        assert_eq!(program.models[0].validations, vec![updated_validation]);

        program.apply_patch_batch(&GraphPatchBatch {
            base_version: 0,
            origin: PatchOrigin::Studio,
            patches: vec![
                GraphPatch::Delete {
                    target_id: query_id,
                },
                GraphPatch::Delete {
                    target_id: validation_id,
                },
            ],
        })?;
        assert!(program.models[0].queries.is_empty());
        assert!(program.models[0].validations.is_empty());
        Ok(())
    }
}
