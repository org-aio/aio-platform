fn validate_field_options(field: &FieldDefinition, diagnostics: &mut Vec<Diagnostic>) {
    let validation = &field.options.validation;
    if validation
        .min_length
        .zip(validation.max_length)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        diagnostics.push(diagnostic(
            "FIELD_LENGTH_RANGE_INVALID",
            CompilerStage::Schema,
            format!("字段 {} 的最小长度不能大于最大长度", field.name),
            Some(field.id),
        ));
    }
    if validation
        .minimum
        .zip(validation.maximum)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        diagnostics.push(diagnostic(
            "FIELD_NUMBER_RANGE_INVALID",
            CompilerStage::Schema,
            format!("字段 {} 的最小值不能大于最大值", field.name),
            Some(field.id),
        ));
    }
    if let Some(pattern) = validation.pattern.as_deref()
        && let Err(error) = regex::Regex::new(pattern)
    {
        diagnostics.push(diagnostic(
            "FIELD_PATTERN_INVALID",
            CompilerStage::Schema,
            format!("字段 {} 的正则表达式无效: {error}", field.name),
            Some(field.id),
        ));
    }
    if validation
        .min_items
        .zip(validation.max_items)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        diagnostics.push(diagnostic(
            "FIELD_COLLECTION_RANGE_INVALID",
            CompilerStage::Schema,
            format!("字段 {} 的最小集合长度不能大于最大集合长度", field.name),
            Some(field.id),
        ));
    }
    if (validation.min_items.is_some() || validation.max_items.is_some() || validation.unique_items)
        && !matches!(field.value_type, crate::ValueType::List { .. })
    {
        diagnostics.push(diagnostic(
            "FIELD_COLLECTION_VALIDATION_NON_LIST",
            CompilerStage::Schema,
            format!("字段 {} 的集合校验只能用于列表字段", field.name),
            Some(field.id),
        ));
    }
}

fn validate_model_indexes(model: &ModelDefinition, diagnostics: &mut Vec<Diagnostic>) {
    for index in &model.indexes {
        if index.fields.is_empty() {
            diagnostics.push(diagnostic(
                "MODEL_INDEX_FIELDS_EMPTY",
                CompilerStage::Schema,
                format!("模型 {} 的索引至少需要一个字段", model.name),
                Some(index.id),
            ));
            continue;
        }
        let mut seen = BTreeSet::new();
        for field_id in &index.fields {
            if !seen.insert(*field_id) {
                diagnostics.push(diagnostic(
                    "MODEL_INDEX_FIELD_DUPLICATE",
                    CompilerStage::Schema,
                    format!("模型 {} 的索引重复引用字段 {field_id}", model.name),
                    Some(index.id),
                ));
            }
            if model.fields.iter().all(|field| field.id != *field_id) {
                diagnostics.push(diagnostic(
                    "MODEL_INDEX_FIELD_MODEL_MISMATCH",
                    CompilerStage::Linking,
                    format!("索引字段 {field_id} 不属于模型 {}", model.name),
                    Some(index.id),
                ));
            }
        }
    }
}

fn validate_model_audit(model: &ModelDefinition, diagnostics: &mut Vec<Diagnostic>) {
    let mut kinds = BTreeSet::new();
    let mut field_ids = BTreeSet::new();
    for audit_field in &model.audit.fields {
        if !kinds.insert(audit_field.kind) {
            diagnostics.push(diagnostic(
                "MODEL_AUDIT_KIND_DUPLICATE",
                CompilerStage::Schema,
                format!(
                    "模型 {} 重复配置审计角色 {}",
                    model.name,
                    audit_field.kind.label()
                ),
                Some(model.id),
            ));
        }
        if !field_ids.insert(audit_field.field_id) {
            diagnostics.push(diagnostic(
                "MODEL_AUDIT_FIELD_DUPLICATE",
                CompilerStage::Schema,
                format!("模型 {} 的多个审计角色绑定了同一字段", model.name),
                Some(model.id),
            ));
        }
        let Some(field) = model
            .fields
            .iter()
            .find(|field| field.id == audit_field.field_id)
        else {
            diagnostics.push(diagnostic(
                "MODEL_AUDIT_FIELD_MISSING",
                CompilerStage::Linking,
                format!(
                    "模型 {} 的审计角色 {} 未绑定有效字段",
                    model.name,
                    audit_field.kind.label()
                ),
                Some(model.id),
            ));
            continue;
        };
        if field.value_type != audit_field.kind.default_value_type() {
            diagnostics.push(diagnostic(
                "MODEL_AUDIT_FIELD_TYPE_INVALID",
                CompilerStage::Types,
                format!(
                    "模型 {} 的审计角色 {} 字段类型不匹配",
                    model.name,
                    audit_field.kind.label()
                ),
                Some(field.id),
            ));
        }
    }
}

fn validate_model_relations(definition: &ProgramDefinition, diagnostics: &mut Vec<Diagnostic>) {
    for model in &definition.models {
        for field in &model.fields {
            let Some(relation) = &field.relation else {
                continue;
            };
            let Some(target_model) = definition
                .models
                .iter()
                .find(|candidate| candidate.id == relation.target_model_id)
            else {
                diagnostics.push(diagnostic(
                    "RELATION_TARGET_MODEL_MISSING",
                    CompilerStage::Linking,
                    format!("字段 {} 的关联模型不存在", field.name),
                    Some(field.id),
                ));
                continue;
            };
            let Some(target_field) = target_model
                .fields
                .iter()
                .find(|candidate| candidate.id == relation.target_field_id)
            else {
                diagnostics.push(diagnostic(
                    "RELATION_TARGET_FIELD_MISSING",
                    CompilerStage::Linking,
                    format!(
                        "字段 {} 的对端字段不属于模型 {}",
                        field.name, target_model.name
                    ),
                    Some(field.id),
                ));
                continue;
            };
            let expected_type = relation_value_type(relation.kind, target_model.id);
            if field.value_type != expected_type {
                diagnostics.push(diagnostic(
                    "RELATION_VALUE_TYPE_MISMATCH",
                    CompilerStage::Types,
                    format!("字段 {} 的类型必须与关联基数一致", field.name),
                    Some(field.id),
                ));
            }
            let Some(opposite) = &target_field.relation else {
                diagnostics.push(diagnostic(
                    "RELATION_OPPOSITE_MISSING",
                    CompilerStage::Linking,
                    format!(
                        "字段 {} 未被对端字段 {} 反向声明",
                        field.name, target_field.name
                    ),
                    Some(field.id),
                ));
                continue;
            };
            if opposite.target_model_id != model.id
                || opposite.target_field_id != field.id
                || opposite.kind != relation.kind.opposite()
            {
                diagnostics.push(diagnostic(
                    "RELATION_OPPOSITE_MISMATCH",
                    CompilerStage::Linking,
                    format!(
                        "字段 {} 与 {} 的关联定义不一致",
                        field.name, target_field.name
                    ),
                    Some(field.id),
                ));
            }
        }
    }
}

fn relation_value_type(kind: crate::RelationKind, target_model_id: SymbolId) -> crate::ValueType {
    let value = crate::ValueType::Object {
        model_id: target_model_id,
    };
    if kind.is_collection() {
        crate::ValueType::List {
            item: Box::new(value),
        }
    } else {
        value
    }
}

fn validate_model_queries(
    definition: &ProgramDefinition,
    model: &ModelDefinition,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut names = BTreeSet::new();
    for query in &model.queries {
        if !data_identifier_is_valid(&query.name) || !names.insert(query.name.as_str()) {
            diagnostics.push(diagnostic(
                "MODEL_QUERY_IDENTIFIER_INVALID",
                CompilerStage::Schema,
                format!("模型 {} 的查询标识无效或重复: {}", model.name, query.name),
                Some(query.id),
            ));
        }
        if query.title.trim().is_empty() || query.conditions.is_empty() {
            diagnostics.push(diagnostic(
                "MODEL_QUERY_INCOMPLETE",
                CompilerStage::Schema,
                format!(
                    "模型 {} 的查询 {} 必须包含标题和条件",
                    model.name, query.name
                ),
                Some(query.id),
            ));
        }
        for condition in &query.conditions {
            validate_query_condition(definition, model, query.id, condition, diagnostics);
        }
    }
}

fn validate_query_condition(
    definition: &ProgramDefinition,
    model: &ModelDefinition,
    query_id: SymbolId,
    condition: &crate::QueryCondition,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (parameter, valid) = match condition {
        crate::QueryCondition::Field {
            field_id,
            parameter,
            ..
        } => (
            parameter,
            model.fields.iter().any(|field| field.id == *field_id),
        ),
        crate::QueryCondition::Relation {
            relation_field_id,
            target_field_id,
            parameter,
            ..
        } => {
            let valid = model
                .fields
                .iter()
                .find(|field| field.id == *relation_field_id)
                .and_then(|field| field.relation.as_ref())
                .and_then(|relation| {
                    definition
                        .models
                        .iter()
                        .find(|candidate| candidate.id == relation.target_model_id)
                })
                .is_some_and(|target_model| {
                    target_model
                        .fields
                        .iter()
                        .any(|field| field.id == *target_field_id)
                });
            (parameter, valid)
        }
    };
    if !valid {
        diagnostics.push(diagnostic(
            "MODEL_QUERY_CONDITION_INVALID",
            CompilerStage::Linking,
            format!("模型 {} 的查询条件引用了无效字段或关联", model.name),
            Some(query_id),
        ));
    }
    if !data_identifier_is_valid(parameter) {
        diagnostics.push(diagnostic(
            "MODEL_QUERY_PARAMETER_INVALID",
            CompilerStage::Schema,
            format!(
                "模型 {} 的查询参数必须是 snake_case: {parameter}",
                model.name
            ),
            Some(query_id),
        ));
    }
}

fn validate_model_validations(model: &ModelDefinition, diagnostics: &mut Vec<Diagnostic>) {
    for validation in &model.validations {
        if validation.message.trim().is_empty() {
            diagnostics.push(diagnostic(
                "MODEL_VALIDATION_MESSAGE_EMPTY",
                CompilerStage::Schema,
                format!("模型 {} 的校验提示不能为空", model.name),
                Some(validation.id),
            ));
        }
        match &validation.rule {
            crate::ModelValidationRule::FieldsRequiredTogether { field_ids }
            | crate::ModelValidationRule::AtLeastOneRequired { field_ids } => {
                validate_validation_fields(model, validation.id, field_ids, diagnostics);
                if field_ids.len() < 2 {
                    diagnostics.push(diagnostic(
                        "MODEL_VALIDATION_FIELD_COUNT_INVALID",
                        CompilerStage::Schema,
                        format!("模型 {} 的联合校验至少需要两个字段", model.name),
                        Some(validation.id),
                    ));
                }
            }
            crate::ModelValidationRule::RequiredWhenPresent {
                field_id,
                when_field_id,
            } => {
                validate_validation_fields(
                    model,
                    validation.id,
                    &[*field_id, *when_field_id],
                    diagnostics,
                );
                if field_id == when_field_id {
                    diagnostics.push(diagnostic(
                        "MODEL_VALIDATION_SELF_DEPENDENCY",
                        CompilerStage::Schema,
                        format!("模型 {} 的条件必填不能引用同一字段", model.name),
                        Some(validation.id),
                    ));
                }
            }
        }
    }
}

fn validate_validation_fields(
    model: &ModelDefinition,
    validation_id: SymbolId,
    field_ids: &[SymbolId],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for field_id in field_ids {
        if !seen.insert(*field_id) || model.fields.iter().all(|field| field.id != *field_id) {
            diagnostics.push(diagnostic(
                "MODEL_VALIDATION_FIELD_INVALID",
                CompilerStage::Linking,
                format!("模型 {} 的校验字段无效或重复: {field_id}", model.name),
                Some(validation_id),
            ));
        }
    }
}

fn validate_page_endpoints(page: &PageDefinition, diagnostics: &mut Vec<Diagnostic>) {
    let mut endpoint_routes = BTreeSet::new();
    for endpoint in &page.endpoints {
        if let Err(error) = validate_route_path(&endpoint.path) {
            diagnostics.push(diagnostic(
                "PAGE_ENDPOINT_PATH_INVALID",
                CompilerStage::Schema,
                error.to_string(),
                Some(endpoint.id),
            ));
        }
        let route_key = (endpoint.method, endpoint.path.as_str());
        if !endpoint_routes.insert(route_key) {
            diagnostics.push(diagnostic(
                "PAGE_ENDPOINT_ROUTE_DUPLICATE",
                CompilerStage::Schema,
                format!(
                    "页面接口路由重复: {} {}",
                    endpoint.method.as_str(),
                    endpoint.path
                ),
                Some(endpoint.id),
            ));
        }
        let mut input_names = BTreeSet::new();
        for input in &endpoint.inputs {
            if !endpoint_identifier_is_valid(&input.name)
                || !input_names.insert(input.name.as_str())
            {
                diagnostics.push(diagnostic(
                    "PAGE_ENDPOINT_INPUT_INVALID",
                    CompilerStage::Schema,
                    format!(
                        "接口 {} {} 的入参标识无效或重复: {}",
                        endpoint.method.as_str(),
                        endpoint.path,
                        input.name
                    ),
                    Some(input.id),
                ));
            }
            if input.location == EndpointInputLocation::Path
                && !endpoint.path.contains(&format!("{{{}}}", input.name))
            {
                diagnostics.push(diagnostic(
                    "PAGE_ENDPOINT_PATH_INPUT_MISSING",
                    CompilerStage::Schema,
                    format!("接口路径缺少参数 {{{}}}: {}", input.name, endpoint.path),
                    Some(input.id),
                ));
            }
        }
        let mut output_names = BTreeSet::new();
        for output in &endpoint.outputs {
            if !endpoint_identifier_is_valid(&output.name)
                || !output_names.insert(output.name.as_str())
            {
                diagnostics.push(diagnostic(
                    "PAGE_ENDPOINT_OUTPUT_INVALID",
                    CompilerStage::Schema,
                    format!(
                        "接口 {} {} 的出参标识无效或重复: {}",
                        endpoint.method.as_str(),
                        endpoint.path,
                        output.name
                    ),
                    Some(output.id),
                ));
            }
        }
    }
}

fn validate_global_endpoint_routes(
    definition: &ProgramDefinition,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut routes = BTreeMap::new();
    for page in &definition.pages {
        for endpoint in compile_page_endpoints(definition, page) {
            let route_key = (endpoint.method, endpoint.path.clone());
            let symbol_id = page
                .endpoints
                .iter()
                .find(|definition| definition.id.to_string() == endpoint.id)
                .map_or(page.id, |definition| definition.id);
            if let Some((existing_page, existing_endpoint, existing_source)) = routes.insert(
                route_key,
                (page.title.clone(), endpoint.title.clone(), endpoint.source),
            ) {
                if existing_source == PageEndpointSource::BuiltIn
                    && endpoint.source == PageEndpointSource::BuiltIn
                {
                    continue;
                }
                diagnostics.push(diagnostic(
                    "PAGE_ENDPOINT_ROUTE_DUPLICATE_GLOBAL",
                    CompilerStage::Schema,
                    format!(
                        "全局接口路由重复: {} {}（{} / {} 与 {} / {}）",
                        endpoint.method.as_str(),
                        endpoint.path,
                        existing_page,
                        existing_endpoint,
                        page.title,
                        endpoint.title
                    ),
                    Some(symbol_id),
                ));
            }
        }
    }
}
