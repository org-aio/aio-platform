fn diagnostic(
    code: &str,
    stage: CompilerStage,
    message: impl Into<String>,
    symbol_id: Option<SymbolId>,
) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        symbol_id,
        stage,
    }
}

fn insert_symbol(
    symbols: &mut BTreeSet<SymbolId>,
    id: SymbolId,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !symbols.insert(id) {
        diagnostics.push(diagnostic(
            "SYMBOL_DUPLICATE",
            CompilerStage::Symbols,
            format!("SymbolId 重复: {id}"),
            Some(id),
        ));
    }
}

fn check_state(id: SymbolId, state: &DefinitionState, diagnostics: &mut Vec<Diagnostic>) {
    if state.is_known() {
        return;
    }
    diagnostics.push(diagnostic(
        "DEFINITION_INCOMPLETE",
        CompilerStage::Symbols,
        format!("可达声明尚不完备: {state:?}"),
        Some(id),
    ));
}

fn check_reference(
    reference: SymbolId,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
    owner: SymbolId,
) {
    if !symbols.contains(&reference) {
        diagnostics.push(diagnostic(
            "SYMBOL_UNRESOLVED",
            CompilerStage::Symbols,
            format!("引用的符号不存在: {reference}"),
            Some(owner),
        ));
    }
}

fn collect_menu_symbols(
    menu: &crate::MenuDefinition,
    reachable: bool,
    symbols: &mut BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    insert_symbol(symbols, menu.id, diagnostics);
    let reachable = reachable && menu.enabled;
    if reachable {
        check_state(menu.id, &menu.state, diagnostics);
    }
    for child in &menu.children {
        collect_menu_symbols(child, reachable, symbols, diagnostics);
    }
}

fn validate_menu_references(
    menu: &crate::MenuDefinition,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !menu.enabled {
        return;
    }
    if let Some(page_id) = menu.page_id {
        check_reference(page_id, symbols, diagnostics, menu.id);
    }
    for permission in &menu.required_permissions {
        check_reference(*permission, symbols, diagnostics, menu.id);
    }
    for access in [
        &menu.row_actions.detail,
        &menu.row_actions.edit,
        &menu.row_actions.delete,
    ] {
        if let crate::MenuActionAccess::Permission { permission_id } = access {
            check_reference(*permission_id, symbols, diagnostics, menu.id);
        }
    }
    for child in &menu.children {
        validate_menu_references(child, symbols, diagnostics);
    }
}

fn published_menus(menus: &[crate::MenuDefinition]) -> Vec<crate::MenuDefinition> {
    menus
        .iter()
        .filter(|menu| menu.enabled)
        .cloned()
        .map(|mut menu| {
            menu.children = published_menus(&menu.children);
            menu
        })
        .collect()
}

fn validate_page_references(
    page: &PageDefinition,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut references = Vec::new();
    match &page.renderer {
        PageRendererDefinition::ConventionFile | PageRendererDefinition::MenuTree => {}
        PageRendererDefinition::CrudTable { table } => {
            collect_table_references(table, &mut references)
        }
        PageRendererDefinition::TreeTable { tree, table } => {
            collect_table_references(table, &mut references);
            references.extend(
                [
                    tree.model_id,
                    tree.label_field_id,
                    tree.parent_field_id,
                    tree.table_relation_field_id,
                ]
                .into_iter()
                .flatten(),
            );
        }
    }
    for endpoint in &page.endpoints {
        for value_type in endpoint
            .inputs
            .iter()
            .map(|input| &input.value_type)
            .chain(endpoint.outputs.iter().map(|output| &output.value_type))
        {
            collect_value_type_references(value_type, &mut references);
        }
    }
    for reference in references {
        check_reference(reference, symbols, diagnostics, page.id);
    }
}

fn collect_value_type_references(value_type: &crate::ValueType, references: &mut Vec<SymbolId>) {
    match value_type {
        crate::ValueType::Object { model_id } => references.push(*model_id),
        crate::ValueType::List { item } => collect_value_type_references(item, references),
        crate::ValueType::Optional { value } => collect_value_type_references(value, references),
        crate::ValueType::Any
        | crate::ValueType::Null
        | crate::ValueType::Boolean
        | crate::ValueType::Integer
        | crate::ValueType::Decimal
        | crate::ValueType::Text
        | crate::ValueType::TimestampMs
        | crate::ValueType::File => {}
    }
}

fn validate_value_type_references(
    value_type: &crate::ValueType,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
    owner: SymbolId,
) {
    let mut references = Vec::new();
    collect_value_type_references(value_type, &mut references);
    for reference in references {
        check_reference(reference, symbols, diagnostics, owner);
    }
}

fn collect_table_references(table: &TableDefinition, references: &mut Vec<SymbolId>) {
    references.extend(table.model_id);
}

fn validate_page_renderer(
    definition: &ProgramDefinition,
    page: &PageDefinition,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &page.renderer {
        PageRendererDefinition::ConventionFile | PageRendererDefinition::MenuTree => {}
        PageRendererDefinition::CrudTable { table } => {
            validate_table(definition, page.id, table, diagnostics);
        }
        PageRendererDefinition::TreeTable { tree, table } => {
            validate_table(definition, page.id, table, diagnostics);
            let Some(tree_model_id) = tree.model_id else {
                diagnostics.push(diagnostic(
                    "TREE_MODEL_REQUIRED",
                    CompilerStage::Linking,
                    "左树右表页面必须选择树模型",
                    Some(page.id),
                ));
                return;
            };
            validate_model_fields(
                definition,
                page.id,
                tree_model_id,
                [tree.label_field_id, tree.parent_field_id]
                    .into_iter()
                    .flatten(),
                diagnostics,
            );
            let Some(table_model_id) = table.model_id else {
                return;
            };
            validate_model_fields(
                definition,
                page.id,
                table_model_id,
                tree.table_relation_field_id,
                diagnostics,
            );
            if tree.label_field_id.is_none() || tree.table_relation_field_id.is_none() {
                diagnostics.push(diagnostic(
                    "TREE_FIELDS_REQUIRED",
                    CompilerStage::Linking,
                    "左树右表页面必须选择树标题字段和表关联字段",
                    Some(page.id),
                ));
            }
        }
    }
}

fn validate_table(
    definition: &ProgramDefinition,
    page_id: SymbolId,
    table: &TableDefinition,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !(1..=200).contains(&table.page_size) {
        diagnostics.push(diagnostic(
            "TABLE_PAGE_SIZE_INVALID",
            CompilerStage::Bounds,
            "表格每页条数必须在 1..=200",
            Some(page_id),
        ));
    }
    let Some(model_id) = table.model_id else {
        diagnostics.push(diagnostic(
            "TABLE_MODEL_REQUIRED",
            CompilerStage::Linking,
            "表格页面必须选择数据模型",
            Some(page_id),
        ));
        return;
    };
    validate_model_fields(
        definition,
        page_id,
        model_id,
        std::iter::empty(),
        diagnostics,
    );
}

fn validate_model_fields(
    definition: &ProgramDefinition,
    page_id: SymbolId,
    model_id: SymbolId,
    fields: impl IntoIterator<Item = SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(model) = definition.models.iter().find(|model| model.id == model_id) else {
        return;
    };
    for field_id in fields {
        if model.fields.iter().all(|field| field.id != field_id) {
            diagnostics.push(diagnostic(
                "PAGE_FIELD_MODEL_MISMATCH",
                CompilerStage::Linking,
                format!("字段 {field_id} 不属于模型 {}", model.name),
                Some(page_id),
            ));
        }
    }
}

fn page_model_dependencies(page: &PageDefinition) -> Vec<SymbolId> {
    let mut values = BTreeSet::new();
    match &page.renderer {
        PageRendererDefinition::ConventionFile | PageRendererDefinition::MenuTree => {}
        PageRendererDefinition::CrudTable { table } => {
            values.extend(table.model_id);
        }
        PageRendererDefinition::TreeTable { tree, table } => {
            values.extend(tree.model_id);
            values.extend(table.model_id);
        }
    }
    values.into_iter().collect()
}

fn validate_function_references(
    function: &FunctionDefinition,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for permission in &function.required_permissions {
        check_reference(*permission, symbols, diagnostics, function.id);
    }
    for port in function.inputs.iter().chain(&function.outputs) {
        validate_value_type_references(&port.value_type, symbols, diagnostics, port.id);
    }
    for edge in &function.graph.edges {
        check_reference(edge.from_node, symbols, diagnostics, edge.id);
        check_reference(edge.to_node, symbols, diagnostics, edge.id);
    }
    for node in &function.graph.nodes {
        match &node.kind {
            FunctionNodeKind::Constant { value_type, .. } => {
                validate_value_type_references(value_type, symbols, diagnostics, node.id)
            }
            FunctionNodeKind::Input { port_id } | FunctionNodeKind::Output { port_id } => {
                check_reference(*port_id, symbols, diagnostics, node.id)
            }
            FunctionNodeKind::Object { fields } => {
                for (field_id, value_node_id) in fields {
                    check_reference(*field_id, symbols, diagnostics, node.id);
                    check_reference(*value_node_id, symbols, diagnostics, node.id);
                }
            }
            FunctionNodeKind::List { items } => {
                for item_node_id in items {
                    check_reference(*item_node_id, symbols, diagnostics, node.id);
                }
            }
            FunctionNodeKind::FieldAccess { object, field_id } => {
                check_reference(*object, symbols, diagnostics, node.id);
                check_reference(*field_id, symbols, diagnostics, node.id);
            }
            FunctionNodeKind::Format { values, .. } => {
                for value_node_id in values {
                    check_reference(*value_node_id, symbols, diagnostics, node.id);
                }
            }
            FunctionNodeKind::ValidateForm { rules } => {
                for rule in rules {
                    check_reference(rule.field_id, symbols, diagnostics, node.id);
                }
            }
            FunctionNodeKind::ForEach {
                body_function_id, ..
            } => check_reference(*body_function_id, symbols, diagnostics, node.id),
            FunctionNodeKind::Navigate { route_id } => {
                check_reference(*route_id, symbols, diagnostics, node.id)
            }
            FunctionNodeKind::CreateRecord { model_id }
            | FunctionNodeKind::ReadRecord { model_id }
            | FunctionNodeKind::UpdateRecord { model_id }
            | FunctionNodeKind::DeleteRecord { model_id }
            | FunctionNodeKind::QueryRecords { model_id, .. } => {
                check_reference(*model_id, symbols, diagnostics, node.id)
            }
            _ => {}
        }
    }
}

fn function_effects(
    function: &FunctionDefinition,
    capabilities: &CapabilityCatalog,
) -> BTreeSet<EffectKind> {
    function
        .graph
        .nodes
        .iter()
        .flat_map(|node| node_effects(&node.kind, capabilities))
        .collect()
}

fn node_effects(kind: &FunctionNodeKind, capabilities: &CapabilityCatalog) -> Vec<EffectKind> {
    match kind {
        FunctionNodeKind::Navigate { .. } => vec![EffectKind::Navigation],
        FunctionNodeKind::Confirm { .. } | FunctionNodeKind::Notify { .. } => {
            vec![EffectKind::UserPrompt]
        }
        FunctionNodeKind::ReadRecord { .. } | FunctionNodeKind::QueryRecords { .. } => {
            vec![EffectKind::DatabaseRead]
        }
        FunctionNodeKind::CreateRecord { .. }
        | FunctionNodeKind::UpdateRecord { .. }
        | FunctionNodeKind::DeleteRecord { .. } => vec![EffectKind::DatabaseWrite],
        FunctionNodeKind::Capability {
            capability_id,
            operation,
        } => capabilities
            .capabilities
            .get(capability_id)
            .and_then(|value| value.operations.get(operation))
            .map(|value| value.effects.clone())
            .unwrap_or_else(|| vec![EffectKind::Capability]),
        _ => Vec::new(),
    }
}

fn is_server_effect(effect: &EffectKind) -> bool {
    matches!(
        effect,
        EffectKind::DatabaseRead
            | EffectKind::DatabaseWrite
            | EffectKind::Secret
            | EffectKind::Capability
    )
}

fn reaches(
    start: SymbolId,
    target: SymbolId,
    graph: &BTreeMap<SymbolId, Vec<SymbolId>>,
    visited: &mut BTreeSet<SymbolId>,
    skip_initial: bool,
) -> bool {
    if !skip_initial && start == target {
        return true;
    }
    if !visited.insert(start) {
        return false;
    }
    graph.get(&start).is_some_and(|next| {
        next.iter()
            .any(|value| reaches(*value, target, graph, visited, false))
    })
}

