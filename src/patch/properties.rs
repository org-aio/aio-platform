impl ProgramDefinition {
    fn set_property(
        &mut self,
        target_id: SymbolId,
        property: &EditableProperty,
        value: &Value,
    ) -> Result<(), PatchError> {
        match property {
            EditableProperty::ApplicationTargets => {
                if target_id != self.id {
                    return Err(PatchError::TargetNotFound(target_id));
                }
                let targets =
                    serde_json::from_value::<std::collections::BTreeSet<ApplicationTarget>>(
                        value.clone(),
                    )
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if targets.is_empty() {
                    return Err(PatchError::InvalidValue(
                        "应用至少需要一个客户端发布目标".to_owned(),
                    ));
                }
                self.application_targets = targets;
                Ok(())
            }
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
                let replacement = serde_json::from_value::<PageEndpointDefinition>(value.clone())
                    .map_err(|error| PatchError::InvalidValue(error.to_string()))?;
                if replacement.id != target_id {
                    return Err(PatchError::InvalidValue(
                        "页面接口更新不能改变 SymbolId".to_owned(),
                    ));
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
            EditableProperty::FieldListVisible
            | EditableProperty::FieldDetailVisible
            | EditableProperty::FieldFormVisible
            | EditableProperty::FieldFormEditable
            | EditableProperty::FieldFilterable
            | EditableProperty::FieldSortable
            | EditableProperty::FieldUnique => {
                let field = self
                    .models
                    .iter_mut()
                    .flat_map(|model| &mut model.fields)
                    .find(|item| item.id == target_id)
                    .ok_or(PatchError::TargetNotFound(target_id))?;
                let enabled = value.as_bool().ok_or_else(|| {
                    PatchError::InvalidValue("字段开关属性必须是布尔值".to_owned())
                })?;
                match property {
                    EditableProperty::FieldListVisible => field.options.list_visible = enabled,
                    EditableProperty::FieldDetailVisible => field.options.detail_visible = enabled,
                    EditableProperty::FieldFormVisible => field.options.form_visible = enabled,
                    EditableProperty::FieldFormEditable => field.options.form_editable = enabled,
                    EditableProperty::FieldFilterable => field.options.filterable = enabled,
                    EditableProperty::FieldSortable => field.options.sortable = enabled,
                    EditableProperty::FieldUnique => field.options.unique = enabled,
                    _ => unreachable!(),
                }
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
