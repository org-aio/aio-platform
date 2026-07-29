use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ApplicationImage, BytecodeInstruction, BytecodeSegment, CapabilityCatalog, ChildrenConstraint,
    CompiledDataSource, CompiledExpressionIndex, CompiledModel, CompiledRoute, ComponentCatalog,
    ComponentNode, DefinitionState, EffectKind, FunctionDefinition, FunctionNode, FunctionNodeKind,
    GraphEdge, ImageTarget, Instruction, ModelDefinition, PROGRAM_SCHEMA_VERSION,
    ProgramDefinition, RenderNode, RenderPlan, SymbolId, validate_route_path,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<SymbolId>,
    #[serde(default)]
    pub stage: CompilerStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerStage {
    #[default]
    Schema,
    Symbols,
    Types,
    Linking,
    Effects,
    Bounds,
    Dependencies,
    Optimization,
    QueryPushdown,
    Lowering,
    SmokeTest,
}

#[derive(Clone, Debug)]
pub struct CompileFailure {
    pub diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Display for CompileFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "程序编译失败，共 {} 条诊断",
            self.diagnostics.len()
        )
    }
}

impl std::error::Error for CompileFailure {}

/// 固定阶段、确定性输出的 ProgramGraph 编译器。
pub struct ProgramCompiler<'a> {
    compiler_version: &'a str,
    components: &'a ComponentCatalog,
    capabilities: &'a CapabilityCatalog,
}

impl<'a> ProgramCompiler<'a> {
    #[must_use]
    pub fn new(
        compiler_version: &'a str,
        components: &'a ComponentCatalog,
        capabilities: &'a CapabilityCatalog,
    ) -> Self {
        Self {
            compiler_version,
            components,
            capabilities,
        }
    }

    pub fn compile(
        &self,
        definition: &ProgramDefinition,
        revision_id: impl Into<String>,
        target: ImageTarget,
    ) -> Result<ApplicationImage, CompileFailure> {
        let mut diagnostics = Vec::new();
        self.validate_schema(definition, &mut diagnostics);
        let symbols = self.resolve_symbols(definition, &mut diagnostics);
        self.infer_types(definition, &mut diagnostics);
        self.link_components_and_capabilities(definition, &symbols, &mut diagnostics);
        self.check_effects_and_permissions(definition, &mut diagnostics);
        self.check_loop_bounds(definition, &mut diagnostics);
        let dependencies = self.analyze_dependencies(definition, &symbols, &mut diagnostics);
        self.fold_constants(definition, &mut diagnostics);
        let models = self.push_down_queries(&definition.models, &mut diagnostics);

        if diagnostics
            .iter()
            .any(|value| value.severity == DiagnosticSeverity::Error)
        {
            return Err(CompileFailure { diagnostics });
        }

        let (pages, client_functions, server_functions, routes) =
            self.lower(definition, &mut diagnostics);
        self.smoke_test(definition, &pages, &routes, &mut diagnostics);
        if diagnostics
            .iter()
            .any(|value| value.severity == DiagnosticSeverity::Error)
        {
            return Err(CompileFailure { diagnostics });
        }

        let content_hash = content_hash(definition).map_err(|error| CompileFailure {
            diagnostics: vec![diagnostic(
                "PROGRAM_HASH_FAILED",
                CompilerStage::Lowering,
                error.to_string(),
                None,
            )],
        })?;
        Ok(ApplicationImage {
            schema_version: PROGRAM_SCHEMA_VERSION,
            compiler_version: self.compiler_version.to_owned(),
            content_hash,
            application_id: definition.id,
            name: definition.name.clone(),
            title: definition.title.clone(),
            revision_id: revision_id.into(),
            target,
            menus: published_menus(&definition.menus),
            permissions: definition.permissions.clone(),
            pages,
            client_functions,
            server_functions,
            models,
            routes,
            dependencies,
        })
    }

    fn validate_schema(&self, definition: &ProgramDefinition, diagnostics: &mut Vec<Diagnostic>) {
        if definition.schema_version != PROGRAM_SCHEMA_VERSION {
            diagnostics.push(diagnostic(
                "PROGRAM_SCHEMA_VERSION",
                CompilerStage::Schema,
                format!(
                    "程序协议版本 {} 与编译器版本 {} 不一致",
                    definition.schema_version, PROGRAM_SCHEMA_VERSION
                ),
                Some(definition.id),
            ));
        }
        if definition.name.trim().is_empty() || definition.title.trim().is_empty() {
            diagnostics.push(diagnostic(
                "PROGRAM_NAME_EMPTY",
                CompilerStage::Schema,
                "程序名称与标题不能为空",
                Some(definition.id),
            ));
        }
        let mut route_paths = BTreeSet::new();
        for route in &definition.routes {
            if let Err(error) = validate_route_path(&route.path) {
                diagnostics.push(diagnostic(
                    "ROUTE_PATH_INVALID",
                    CompilerStage::Schema,
                    error.to_string(),
                    Some(route.id),
                ));
            }
            if !route_paths.insert(route.path.as_str()) {
                diagnostics.push(diagnostic(
                    "ROUTE_PATH_DUPLICATE",
                    CompilerStage::Schema,
                    format!("路由路径重复: {}", route.path),
                    Some(route.id),
                ));
            }
        }
        let mut model_names = BTreeSet::new();
        for model in &definition.models {
            if !is_data_identifier(&model.name) {
                diagnostics.push(diagnostic(
                    "MODEL_IDENTIFIER_INVALID",
                    CompilerStage::Schema,
                    format!("模型标识必须是 snake_case: {}", model.name),
                    Some(model.id),
                ));
            }
            if !model_names.insert(model.name.as_str()) {
                diagnostics.push(diagnostic(
                    "MODEL_IDENTIFIER_DUPLICATE",
                    CompilerStage::Schema,
                    format!("模型标识重复: {}", model.name),
                    Some(model.id),
                ));
            }
            let mut field_names = BTreeSet::new();
            for field in &model.fields {
                if !is_data_identifier(&field.name) {
                    diagnostics.push(diagnostic(
                        "FIELD_IDENTIFIER_INVALID",
                        CompilerStage::Schema,
                        format!("字段标识必须是 snake_case: {}", field.name),
                        Some(field.id),
                    ));
                }
                if !field_names.insert(field.name.as_str()) {
                    diagnostics.push(diagnostic(
                        "FIELD_IDENTIFIER_DUPLICATE",
                        CompilerStage::Schema,
                        format!("字段标识重复: {}.{}", model.name, field.name),
                        Some(field.id),
                    ));
                }
            }
        }
    }

    fn resolve_symbols(
        &self,
        definition: &ProgramDefinition,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> BTreeSet<SymbolId> {
        let mut symbols = BTreeSet::new();
        insert_symbol(&mut symbols, definition.id, diagnostics);
        for menu in &definition.menus {
            collect_menu_symbols(menu, true, &mut symbols, diagnostics);
        }
        for model in &definition.models {
            insert_symbol(&mut symbols, model.id, diagnostics);
            check_state(model.id, &model.state, diagnostics);
            for field in &model.fields {
                insert_symbol(&mut symbols, field.id, diagnostics);
                check_state(field.id, &field.state, diagnostics);
            }
            for index in &model.indexes {
                insert_symbol(&mut symbols, index.id, diagnostics);
            }
        }
        for page in &definition.pages {
            insert_symbol(&mut symbols, page.id, diagnostics);
            check_state(page.id, &page.state, diagnostics);
            collect_component_symbols(&page.root, &mut symbols, diagnostics);
            for state in &page.page_state {
                insert_symbol(&mut symbols, state.id, diagnostics);
            }
            for source in &page.data_sources {
                insert_symbol(&mut symbols, source.id, diagnostics);
            }
        }
        for function in &definition.functions {
            insert_symbol(&mut symbols, function.id, diagnostics);
            check_state(function.id, &function.state, diagnostics);
            for port in function.inputs.iter().chain(&function.outputs) {
                insert_symbol(&mut symbols, port.id, diagnostics);
            }
            for node in &function.graph.nodes {
                insert_symbol(&mut symbols, node.id, diagnostics);
                check_state(node.id, &node.state, diagnostics);
            }
            for edge in &function.graph.edges {
                insert_symbol(&mut symbols, edge.id, diagnostics);
            }
        }
        for route in &definition.routes {
            insert_symbol(&mut symbols, route.id, diagnostics);
            check_state(route.id, &route.state, diagnostics);
        }
        for permission in &definition.permissions {
            insert_symbol(&mut symbols, permission.id, diagnostics);
        }

        for menu in &definition.menus {
            validate_menu_references(menu, &symbols, diagnostics);
        }
        for model in &definition.models {
            for field in &model.fields {
                if let Some(relation) = field.relation_model_id {
                    check_reference(relation, &symbols, diagnostics, field.id);
                }
            }
        }
        for page in &definition.pages {
            validate_component_references(&page.root, &symbols, diagnostics);
            for source in &page.data_sources {
                check_reference(source.function_id, &symbols, diagnostics, source.id);
            }
        }
        for route in &definition.routes {
            check_reference(route.page_id, &symbols, diagnostics, route.id);
        }
        for function in &definition.functions {
            validate_function_references(function, &symbols, diagnostics);
        }
        symbols
    }

    fn infer_types(&self, definition: &ProgramDefinition, diagnostics: &mut Vec<Diagnostic>) {
        for function in &definition.functions {
            let nodes = function
                .graph
                .nodes
                .iter()
                .map(|node| (node.id, node))
                .collect::<BTreeMap<_, _>>();
            for edge in &function.graph.edges {
                let Some(from) = nodes.get(&edge.from_node) else {
                    continue;
                };
                let Some(to) = nodes.get(&edge.to_node) else {
                    continue;
                };
                if !ports_are_compatible(from, to) {
                    diagnostics.push(diagnostic(
                        "GRAPH_TYPE_MISMATCH",
                        CompilerStage::Types,
                        format!("节点 {} 与 {} 的端口类型不兼容", from.name, to.name),
                        Some(edge.id),
                    ));
                }
            }
            if ordered_nodes(function).len() != function.graph.nodes.len() {
                diagnostics.push(diagnostic(
                    "GRAPH_CYCLE_FORBIDDEN",
                    CompilerStage::Types,
                    format!("函数 {} 的节点图存在环", function.name),
                    Some(function.id),
                ));
            }
        }
    }

    fn link_components_and_capabilities(
        &self,
        definition: &ProgramDefinition,
        symbols: &BTreeSet<SymbolId>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for page in &definition.pages {
            self.link_component(&page.root, symbols, diagnostics);
        }
        for function in &definition.functions {
            for node in &function.graph.nodes {
                if let FunctionNodeKind::Capability {
                    capability_id,
                    operation,
                } = &node.kind
                {
                    let linked = self
                        .capabilities
                        .capabilities
                        .get(capability_id)
                        .and_then(|capability| capability.operations.get(operation));
                    if linked.is_none() {
                        diagnostics.push(diagnostic(
                            "CAPABILITY_NOT_LINKED",
                            CompilerStage::Linking,
                            format!("Capability 未注册: {capability_id}.{operation}"),
                            Some(node.id),
                        ));
                    }
                }
            }
        }
    }

    fn link_component(
        &self,
        node: &ComponentNode,
        symbols: &BTreeSet<SymbolId>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let Some(contract) = self.components.components.get(&node.component) else {
            diagnostics.push(diagnostic(
                "COMPONENT_NOT_LINKED",
                CompilerStage::Linking,
                format!("组件未由 Rudi Provider 注册: {}", node.component),
                Some(node.id),
            ));
            return;
        };
        for property in node.properties.keys() {
            if !contract.properties.contains_key(property) {
                diagnostics.push(diagnostic(
                    "COMPONENT_PROPERTY_UNKNOWN",
                    CompilerStage::Linking,
                    format!("组件 {} 不支持属性 {property}", node.component),
                    Some(node.id),
                ));
            }
        }
        for (name, property) in &contract.properties {
            if property.required && !node.properties.contains_key(name) {
                diagnostics.push(diagnostic(
                    "COMPONENT_PROPERTY_REQUIRED",
                    CompilerStage::Linking,
                    format!("组件 {} 缺少必填属性 {name}", node.component),
                    Some(node.id),
                ));
            }
            if let Some(crate::PropertyValue::Literal { value }) = node.properties.get(name) {
                if !literal_matches_type(value, &property.value_type) {
                    diagnostics.push(diagnostic(
                        "COMPONENT_PROPERTY_TYPE",
                        CompilerStage::Types,
                        format!("组件 {} 的属性 {name} 类型不匹配", node.component),
                        Some(node.id),
                    ));
                }
                if !property.choices.is_empty()
                    && value
                        .as_str()
                        .is_none_or(|value| !property.choices.iter().any(|choice| choice == value))
                {
                    diagnostics.push(diagnostic(
                        "COMPONENT_PROPERTY_CHOICE",
                        CompilerStage::Types,
                        format!("组件 {} 的属性 {name} 选项无效", node.component),
                        Some(node.id),
                    ));
                }
            }
        }
        for (event, function_id) in &node.events {
            if !contract.events.contains_key(event) {
                diagnostics.push(diagnostic(
                    "COMPONENT_EVENT_UNKNOWN",
                    CompilerStage::Linking,
                    format!("组件 {} 不支持事件 {event}", node.component),
                    Some(node.id),
                ));
            }
            check_reference(*function_id, symbols, diagnostics, node.id);
        }
        match &contract.children {
            ChildrenConstraint::None if !node.children.is_empty() => diagnostics.push(diagnostic(
                "COMPONENT_CHILDREN_FORBIDDEN",
                CompilerStage::Linking,
                format!("组件 {} 不允许子节点", node.component),
                Some(node.id),
            )),
            ChildrenConstraint::Range { minimum, maximum }
                if node.children.len() < *minimum as usize
                    || node.children.len() > *maximum as usize =>
            {
                diagnostics.push(diagnostic(
                    "COMPONENT_CHILDREN_RANGE",
                    CompilerStage::Linking,
                    format!("组件 {} 的子节点数量不在允许范围内", node.component),
                    Some(node.id),
                ));
            }
            ChildrenConstraint::Components { canonical_ids }
                if node
                    .children
                    .iter()
                    .any(|child| !canonical_ids.contains(&child.component)) =>
            {
                diagnostics.push(diagnostic(
                    "COMPONENT_CHILD_TYPE",
                    CompilerStage::Linking,
                    format!("组件 {} 包含不允许的子组件", node.component),
                    Some(node.id),
                ));
            }
            _ => {}
        }
        for child in &node.children {
            self.link_component(child, symbols, diagnostics);
        }
    }

    fn check_effects_and_permissions(
        &self,
        definition: &ProgramDefinition,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let permissions = definition
            .permissions
            .iter()
            .map(|value| (value.id, value))
            .collect::<BTreeMap<_, _>>();
        for function in &definition.functions {
            let effects = function_effects(function, self.capabilities);
            let allowed = function
                .required_permissions
                .iter()
                .filter_map(|id| permissions.get(id))
                .flat_map(|permission| permission.allowed_effects.iter().copied())
                .collect::<BTreeSet<_>>();
            for effect in effects {
                if matches!(effect, EffectKind::ClientState | EffectKind::Navigation)
                    || allowed.contains(&effect)
                {
                    continue;
                }
                diagnostics.push(diagnostic(
                    "EFFECT_PERMISSION_DENIED",
                    CompilerStage::Effects,
                    format!("函数 {} 未声明 {:?} Effect 的权限", function.name, effect),
                    Some(function.id),
                ));
            }
        }
    }

    fn check_loop_bounds(&self, definition: &ProgramDefinition, diagnostics: &mut Vec<Diagnostic>) {
        let call_graph = definition
            .functions
            .iter()
            .map(|function| {
                let calls = function
                    .graph
                    .nodes
                    .iter()
                    .filter_map(|node| match node.kind {
                        FunctionNodeKind::ForEach {
                            body_function_id, ..
                        } => Some(body_function_id),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                (function.id, calls)
            })
            .collect::<BTreeMap<_, _>>();
        for function in &definition.functions {
            for node in &function.graph.nodes {
                if let FunctionNodeKind::ForEach { max_items, .. } = node.kind
                    && (max_items == 0 || max_items > 10_000)
                {
                    diagnostics.push(diagnostic(
                        "LOOP_BOUND_INVALID",
                        CompilerStage::Bounds,
                        "受控遍历上限必须在 1..=10000",
                        Some(node.id),
                    ));
                }
            }
            if reaches(
                function.id,
                function.id,
                &call_graph,
                &mut BTreeSet::new(),
                true,
            ) {
                diagnostics.push(diagnostic(
                    "FUNCTION_RECURSION_FORBIDDEN",
                    CompilerStage::Bounds,
                    format!("函数 {} 形成递归调用", function.name),
                    Some(function.id),
                ));
            }
        }
    }

    fn analyze_dependencies(
        &self,
        definition: &ProgramDefinition,
        _symbols: &BTreeSet<SymbolId>,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> BTreeMap<SymbolId, Vec<SymbolId>> {
        let mut dependencies = BTreeMap::new();
        for page in &definition.pages {
            let mut values = page
                .data_sources
                .iter()
                .map(|source| source.function_id)
                .collect::<BTreeSet<_>>();
            collect_component_dependencies(&page.root, &mut values);
            dependencies.insert(page.id, values.into_iter().collect());
        }
        for function in &definition.functions {
            let values = function
                .graph
                .nodes
                .iter()
                .filter_map(|node| match node.kind {
                    FunctionNodeKind::ForEach {
                        body_function_id, ..
                    } => Some(body_function_id),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            dependencies.insert(function.id, values.into_iter().collect());
        }
        dependencies
    }

    fn fold_constants(&self, _definition: &ProgramDefinition, _diagnostics: &mut Vec<Diagnostic>) {
        // 常量在 lowering 时写入独立池，保持指令确定性。
    }

    fn push_down_queries(
        &self,
        models: &[ModelDefinition],
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> BTreeMap<SymbolId, CompiledModel> {
        models
            .iter()
            .map(|model| {
                let field_slots = model
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(slot, field)| (field.id, slot as u32))
                    .collect::<BTreeMap<_, _>>();
                let field_types = model
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(slot, field)| (slot as u32, field.value_type.clone()))
                    .collect::<BTreeMap<_, _>>();
                let field_names = model
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(slot, field)| (slot as u32, field.name.clone()))
                    .collect::<BTreeMap<_, _>>();
                let expression_indexes = model
                    .indexes
                    .iter()
                    .map(|index| {
                        let slots = index
                            .fields
                            .iter()
                            .filter_map(|field| field_slots.get(field).copied())
                            .collect::<Vec<_>>();
                        let expression = slots
                            .iter()
                            .filter_map(|slot| field_names.get(slot))
                            .map(|field| format!("(payload ->> '{field}')"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        CompiledExpressionIndex {
                            fields: slots,
                            expression,
                        }
                    })
                    .collect();
                (
                    model.id,
                    CompiledModel {
                        id: model.id,
                        name: model.name.clone(),
                        field_slots,
                        field_types,
                        field_names,
                        expression_indexes,
                    },
                )
            })
            .collect()
    }

    fn lower(
        &self,
        definition: &ProgramDefinition,
        _diagnostics: &mut Vec<Diagnostic>,
    ) -> (
        BTreeMap<SymbolId, RenderPlan>,
        BTreeMap<SymbolId, BytecodeSegment>,
        BTreeMap<SymbolId, BytecodeSegment>,
        Vec<CompiledRoute>,
    ) {
        let pages = definition
            .pages
            .iter()
            .map(|page| (page.id, preview_render_plan(page)))
            .collect();
        let mut client_functions = BTreeMap::new();
        let mut server_functions = BTreeMap::new();
        for function in &definition.functions {
            let (client_segment, function_server_segments) =
                lower_function(function, self.capabilities);
            client_functions.insert(function.id, client_segment);
            for segment in function_server_segments {
                server_functions.insert(segment.id, segment);
            }
        }
        let routes = definition
            .routes
            .iter()
            .map(|route| CompiledRoute {
                id: route.id,
                name: route.name.clone(),
                path: route.path.clone(),
                page_id: route.page_id,
                required_permissions: route.required_permissions.clone(),
            })
            .collect();
        (pages, client_functions, server_functions, routes)
    }

    fn smoke_test(
        &self,
        _definition: &ProgramDefinition,
        pages: &BTreeMap<SymbolId, RenderPlan>,
        routes: &[CompiledRoute],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for route in routes {
            if !pages.contains_key(&route.page_id) {
                diagnostics.push(diagnostic(
                    "SMOKE_ROUTE_PAGE_MISSING",
                    CompilerStage::SmokeTest,
                    format!("路由 {} 的页面未进入 RenderPlan", route.path),
                    Some(route.id),
                ));
            }
        }
    }
}

#[must_use]
pub fn preview_render_plan(page: &crate::PageDefinition) -> RenderPlan {
    RenderPlan {
        page_id: page.id,
        name: page.name.clone(),
        title: page.title.clone(),
        root: lower_render_node(&page.root),
        page_state: page
            .page_state
            .iter()
            .map(|value| (value.id, value.initial_value.clone()))
            .collect(),
        data_sources: page
            .data_sources
            .iter()
            .map(|source| CompiledDataSource {
                id: source.id,
                name: source.name.clone(),
                function_id: source.function_id,
                parameters: source.parameters.clone(),
            })
            .collect(),
    }
}

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

fn collect_component_symbols(
    component: &ComponentNode,
    symbols: &mut BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    insert_symbol(symbols, component.id, diagnostics);
    check_state(component.id, &component.state, diagnostics);
    for child in &component.children {
        collect_component_symbols(child, symbols, diagnostics);
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

fn validate_component_references(
    component: &ComponentNode,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for value in component
        .properties
        .values()
        .chain(component.content.iter())
    {
        match value {
            crate::PropertyValue::PageState { state_id } => {
                check_reference(*state_id, symbols, diagnostics, component.id)
            }
            crate::PropertyValue::DataSource { source_id, path } => {
                check_reference(*source_id, symbols, diagnostics, component.id);
                for field_id in path {
                    check_reference(*field_id, symbols, diagnostics, component.id);
                }
            }
            crate::PropertyValue::Literal { .. } | crate::PropertyValue::EventValue { .. } => {}
        }
    }
    for function in component.events.values() {
        check_reference(*function, symbols, diagnostics, component.id);
    }
    for child in &component.children {
        validate_component_references(child, symbols, diagnostics);
    }
}

fn validate_function_references(
    function: &FunctionDefinition,
    symbols: &BTreeSet<SymbolId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for permission in &function.required_permissions {
        check_reference(*permission, symbols, diagnostics, function.id);
    }
    for edge in &function.graph.edges {
        check_reference(edge.from_node, symbols, diagnostics, edge.id);
        check_reference(edge.to_node, symbols, diagnostics, edge.id);
    }
    for node in &function.graph.nodes {
        match node.kind {
            FunctionNodeKind::ForEach {
                body_function_id, ..
            } => check_reference(body_function_id, symbols, diagnostics, node.id),
            FunctionNodeKind::Navigate { route_id } => {
                check_reference(route_id, symbols, diagnostics, node.id)
            }
            FunctionNodeKind::CreateRecord { model_id }
            | FunctionNodeKind::ReadRecord { model_id }
            | FunctionNodeKind::UpdateRecord { model_id }
            | FunctionNodeKind::DeleteRecord { model_id }
            | FunctionNodeKind::QueryRecords { model_id, .. } => {
                check_reference(model_id, symbols, diagnostics, node.id)
            }
            _ => {}
        }
    }
}

fn ports_are_compatible(from: &FunctionNode, to: &FunctionNode) -> bool {
    !matches!(from.kind, FunctionNodeKind::Fail { .. })
        && !matches!(
            to.kind,
            FunctionNodeKind::Constant { .. } | FunctionNodeKind::Input { .. }
        )
}

fn literal_matches_type(value: &serde_json::Value, expected: &crate::ValueType) -> bool {
    match expected {
        crate::ValueType::Any => true,
        crate::ValueType::Null => value.is_null(),
        crate::ValueType::Boolean => value.is_boolean(),
        crate::ValueType::Integer => value.as_i64().is_some(),
        crate::ValueType::Decimal => value.is_number(),
        crate::ValueType::Text | crate::ValueType::File => value.is_string(),
        crate::ValueType::TimestampMs => value.as_i64().is_some(),
        crate::ValueType::Object { .. } => value.is_object(),
        crate::ValueType::List { .. } => value.is_array(),
        crate::ValueType::Optional { value: inner } => {
            value.is_null() || literal_matches_type(value, inner)
        }
    }
}

fn is_data_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'_')
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
        FunctionNodeKind::SetState { .. } | FunctionNodeKind::Refresh { .. } => {
            vec![EffectKind::ClientState]
        }
        FunctionNodeKind::Navigate { .. } => vec![EffectKind::Navigation],
        FunctionNodeKind::Confirm { .. }
        | FunctionNodeKind::OpenDialog { .. }
        | FunctionNodeKind::CloseDialog { .. }
        | FunctionNodeKind::Notify { .. } => vec![EffectKind::UserPrompt],
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

fn collect_component_dependencies(component: &ComponentNode, values: &mut BTreeSet<SymbolId>) {
    values.extend(component.events.values().copied());
    for child in &component.children {
        collect_component_dependencies(child, values);
    }
}

fn lower_render_node(node: &ComponentNode) -> RenderNode {
    RenderNode {
        id: node.id,
        component: node.component.clone(),
        properties: node.properties.clone(),
        content: node.content.clone(),
        events: node.events.clone(),
        children: node.children.iter().map(lower_render_node).collect(),
        style: node.style.clone(),
        responsive_visibility: node
            .style
            .responsive
            .iter()
            .filter_map(|(breakpoint, style)| style.visible.map(|visible| (*breakpoint, visible)))
            .collect(),
    }
}

fn lower_function(
    function: &FunctionDefinition,
    capabilities: &CapabilityCatalog,
) -> (BytecodeSegment, Vec<BytecodeSegment>) {
    let mut constants = Vec::new();
    let mut instructions = Vec::new();
    let mut server_segments = Vec::new();
    let nodes = ordered_nodes(function);
    let slots = nodes
        .iter()
        .enumerate()
        .map(|(slot, node)| (node.id, slot as u32))
        .collect::<BTreeMap<_, _>>();
    let incoming_edges = function.graph.edges.iter().fold(
        BTreeMap::<SymbolId, Vec<&GraphEdge>>::new(),
        |mut values, edge| {
            values.entry(edge.to_node).or_default().push(edge);
            values
        },
    );
    for (slot, node) in nodes.into_iter().enumerate() {
        let slot = slot as u32;
        let lowered = match &node.kind {
            FunctionNodeKind::Constant { value, .. } => {
                let constant = constants.len() as u32;
                constants.push(value.clone());
                Instruction::LoadConstant { slot, constant }
            }
            FunctionNodeKind::Input { port_id } => Instruction::LoadInput {
                slot,
                port_id: *port_id,
            },
            FunctionNodeKind::Object { fields } => Instruction::MakeObject {
                slot,
                fields: fields.keys().copied().collect(),
            },
            FunctionNodeKind::List { items } => Instruction::MakeList {
                slot,
                count: items.len() as u32,
            },
            FunctionNodeKind::FieldAccess { field_id, .. } => Instruction::ReadField {
                slot,
                field_id: *field_id,
            },
            FunctionNodeKind::Format { template, values } => Instruction::Format {
                slot,
                template: template.clone(),
                count: values.len() as u32,
            },
            FunctionNodeKind::Compare { operator } => Instruction::Compare {
                slot,
                operator: format!("{operator:?}").to_lowercase(),
            },
            FunctionNodeKind::Boolean { operator } => Instruction::Boolean {
                slot,
                operator: format!("{operator:?}").to_lowercase(),
            },
            FunctionNodeKind::Math { operator } => Instruction::Math {
                slot,
                operator: format!("{operator:?}").to_lowercase(),
            },
            FunctionNodeKind::Condition => Instruction::Branch {
                condition_slot: slot,
            },
            FunctionNodeKind::ForEach {
                max_items,
                body_function_id,
            } => Instruction::ForEach {
                max_items: *max_items,
                body_function_id: *body_function_id,
            },
            FunctionNodeKind::SetState { state_id } => Instruction::SetState {
                state_id: *state_id,
            },
            FunctionNodeKind::ValidateForm { rules } => Instruction::ValidateForm {
                rule_count: rules.len() as u32,
            },
            FunctionNodeKind::CreateRecord { model_id } => Instruction::CreateRecord {
                model_id: *model_id,
            },
            FunctionNodeKind::ReadRecord { model_id } => Instruction::ReadRecord {
                model_id: *model_id,
            },
            FunctionNodeKind::UpdateRecord { model_id } => Instruction::UpdateRecord {
                model_id: *model_id,
            },
            FunctionNodeKind::DeleteRecord { model_id } => Instruction::DeleteRecord {
                model_id: *model_id,
            },
            FunctionNodeKind::QueryRecords { model_id, limit } => Instruction::QueryRecords {
                model_id: *model_id,
                limit: *limit,
            },
            FunctionNodeKind::Navigate { route_id } => Instruction::Navigate {
                route_id: *route_id,
            },
            FunctionNodeKind::Confirm { .. } => Instruction::Confirm,
            FunctionNodeKind::OpenDialog { component_id } => Instruction::OpenDialog {
                component_id: *component_id,
            },
            FunctionNodeKind::CloseDialog { component_id } => Instruction::CloseDialog {
                component_id: *component_id,
            },
            FunctionNodeKind::Notify { level } => Instruction::Notify {
                level: format!("{level:?}").to_lowercase(),
            },
            FunctionNodeKind::Refresh { source_id } => Instruction::Refresh {
                source_id: *source_id,
            },
            FunctionNodeKind::Capability {
                capability_id,
                operation,
            } => Instruction::InvokeCapability {
                capability_id: capability_id.clone(),
                operation: operation.clone(),
            },
            FunctionNodeKind::Output { .. } | FunctionNodeKind::Return => Instruction::Return,
            FunctionNodeKind::Fail { code } => Instruction::Fail { code: code.clone() },
        };
        let input_slots = incoming_edges
            .get(&node.id)
            .into_iter()
            .flatten()
            .filter_map(|edge| {
                slots
                    .get(&edge.from_node)
                    .copied()
                    .map(|slot| (edge.to_port.clone(), slot))
            })
            .collect();
        let node_effects = node_effects(&node.kind, capabilities);
        let is_server_node = node_effects.iter().any(is_server_effect);
        let instruction = if is_server_node {
            server_segments.push(BytecodeSegment {
                id: node.id,
                name: format!("{}::{}", function.name, node.name),
                input_ports: BTreeMap::from([(node.id, "value".to_owned())]),
                effects: node_effects,
                instructions: vec![
                    BytecodeInstruction {
                        node_id: node.id,
                        input_slots: BTreeMap::new(),
                        output_slot: Some(0),
                        instruction: Instruction::LoadInput {
                            slot: 0,
                            port_id: node.id,
                        },
                    },
                    BytecodeInstruction {
                        node_id: node.id,
                        input_slots: BTreeMap::from([("value".to_owned(), 0)]),
                        output_slot: Some(1),
                        instruction: lowered,
                    },
                ],
                constants: Vec::new(),
            });
            Instruction::InvokeServerSegment {
                segment_id: node.id,
                input_port: node.id,
            }
        } else {
            lowered
        };
        let output_slot = (!matches!(instruction, Instruction::Return | Instruction::Fail { .. }))
            .then_some(slot);
        instructions.push(BytecodeInstruction {
            node_id: node.id,
            input_slots,
            output_slot,
            instruction,
        });
    }
    let client_segment = BytecodeSegment {
        id: function.id,
        name: function.name.clone(),
        input_ports: function
            .inputs
            .iter()
            .map(|port| (port.id, port.name.clone()))
            .collect(),
        effects: function_effects(function, capabilities)
            .into_iter()
            .filter(|effect| !is_server_effect(effect))
            .collect(),
        instructions,
        constants,
    };
    (client_segment, server_segments)
}

fn ordered_nodes(function: &FunctionDefinition) -> Vec<&FunctionNode> {
    let nodes = function
        .graph
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut incoming = nodes
        .keys()
        .map(|id| (*id, 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut outgoing = BTreeMap::<SymbolId, Vec<SymbolId>>::new();
    for edge in &function.graph.edges {
        if nodes.contains_key(&edge.from_node) && nodes.contains_key(&edge.to_node) {
            if let Some(count) = incoming.get_mut(&edge.to_node) {
                *count = count.saturating_add(1);
            }
            outgoing
                .entry(edge.from_node)
                .or_default()
                .push(edge.to_node);
        }
    }
    let mut ready = incoming
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some(id) = ready.pop_first() {
        if let Some(node) = nodes.get(&id) {
            ordered.push(*node);
        }
        for target in outgoing.get(&id).into_iter().flatten() {
            if let Some(count) = incoming.get_mut(target) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.insert(*target);
                }
            }
        }
    }
    ordered
}

pub fn content_hash(definition: &ProgramDefinition) -> Result<String> {
    let bytes = serde_json::to_vec(definition)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::{
        ComponentContract, ComponentStyle, DefinitionState, EffectKind, FunctionGraph,
        FunctionNodeEditor, GraphEdge, MenuDefinition, ModelDefinition, NotificationLevel,
        PageDefinition, PermissionDefinition, ProgramDefinition,
    };

    fn test_catalog() -> ComponentCatalog {
        ComponentCatalog {
            components: BTreeMap::from([(
                "layout.section".to_owned(),
                ComponentContract {
                    canonical_id: "layout.section".to_owned(),
                    properties: BTreeMap::new(),
                    events: BTreeMap::new(),
                    children: ChildrenConstraint::Any,
                },
            )]),
        }
    }

    fn valid_program() -> ProgramDefinition {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let page_id = SymbolId::new();
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
        program.routes.push(crate::RouteDefinition {
            id: SymbolId::new(),
            name: "assets".to_owned(),
            path: "/assets".to_owned(),
            page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });
        program
    }

    #[test]
    fn content_hash_is_deterministic() -> anyhow::Result<()> {
        let program = valid_program();
        assert_eq!(content_hash(&program)?, content_hash(&program)?);
        Ok(())
    }

    #[test]
    fn incomplete_reachable_definition_is_rejected() {
        let mut program = valid_program();
        program.pages[0].state = DefinitionState::Hole {
            expected: "页面标题".to_owned(),
        };
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let compiler = ProgramCompiler::new("test", &components, &capabilities);
        let failure = compiler
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .map(|value| value.diagnostics)
            .unwrap_or_default();
        assert!(
            failure
                .iter()
                .any(|value| value.code == "DEFINITION_INCOMPLETE")
        );
    }

    #[test]
    fn disabled_menu_is_not_published_or_validated_as_reachable() -> anyhow::Result<()> {
        let mut program = valid_program();
        program.menus.push(MenuDefinition {
            id: SymbolId::new(),
            name: "draft-menu".to_owned(),
            title: "未启用菜单".to_owned(),
            state: DefinitionState::Hole {
                expected: "页面绑定".to_owned(),
            },
            icon: None,
            page_id: Some(SymbolId::new()),
            enabled: false,
            children: Vec::new(),
            required_permissions: vec![SymbolId::new()],
        });
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let compiler = ProgramCompiler::new("test", &components, &capabilities);

        let image = compiler.compile(&program, "revision-1", ImageTarget::Universal)?;

        assert!(image.menus.is_empty());
        Ok(())
    }

    #[test]
    fn unknown_component_is_rejected() {
        let mut program = valid_program();
        program.pages[0].root.component = "unknown".to_owned();
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let compiler = ProgramCompiler::new("test", &components, &capabilities);
        let failure = compiler
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .map(|value| value.diagnostics)
            .unwrap_or_default();
        assert!(
            failure
                .iter()
                .any(|value| value.code == "COMPONENT_NOT_LINKED")
        );
    }

    #[test]
    fn constants_are_lowered_into_image() -> anyhow::Result<()> {
        let mut program = valid_program();
        let function_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "answer".to_owned(),
            title: "答案".to_owned(),
            state: DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![FunctionNode {
                    id: SymbolId::new(),
                    name: "value".to_owned(),
                    state: DefinitionState::Known,
                    editor: FunctionNodeEditor::default(),
                    kind: FunctionNodeKind::Constant {
                        value: json!(42),
                        value_type: crate::ValueType::Integer,
                    },
                }],
                edges: Vec::new(),
            },
            required_permissions: Vec::new(),
        });
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let compiler = ProgramCompiler::new("test", &components, &capabilities);
        let image = compiler.compile(&program, "revision-1", ImageTarget::Universal)?;
        let segment = image.client_functions.get(&function_id);
        assert_eq!(
            segment.map(|value| value.constants.as_slice()),
            Some([json!(42)].as_slice())
        );
        Ok(())
    }

    #[test]
    fn server_effect_nodes_are_split_from_client_effects() -> anyhow::Result<()> {
        let mut program = valid_program();
        let model_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            fields: Vec::new(),
            indexes: Vec::new(),
        });
        let permission_id = SymbolId::new();
        program.permissions.push(PermissionDefinition {
            id: permission_id,
            name: "asset-write".to_owned(),
            title: "资产写入".to_owned(),
            allowed_effects: vec![EffectKind::DatabaseWrite, EffectKind::UserPrompt],
        });
        let function_id = SymbolId::new();
        let value_node_id = SymbolId::new();
        let create_node_id = SymbolId::new();
        let notify_node_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "create-asset".to_owned(),
            title: "创建资产".to_owned(),
            state: DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![
                    FunctionNode {
                        id: value_node_id,
                        name: "payload".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Constant {
                            value: json!({"name": "server"}),
                            value_type: crate::ValueType::Any,
                        },
                    },
                    FunctionNode {
                        id: create_node_id,
                        name: "create".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::CreateRecord { model_id },
                    },
                    FunctionNode {
                        id: notify_node_id,
                        name: "notify".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Notify {
                            level: NotificationLevel::Success,
                        },
                    },
                ],
                edges: vec![
                    GraphEdge {
                        id: SymbolId::new(),
                        from_node: value_node_id,
                        from_port: "value".to_owned(),
                        to_node: create_node_id,
                        to_port: "value".to_owned(),
                    },
                    GraphEdge {
                        id: SymbolId::new(),
                        from_node: create_node_id,
                        from_port: "value".to_owned(),
                        to_node: notify_node_id,
                        to_port: "value".to_owned(),
                    },
                ],
            },
            required_permissions: vec![permission_id],
        });
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let image = ProgramCompiler::new("test", &components, &capabilities).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;

        let client = image
            .client_functions
            .get(&function_id)
            .expect("客户端入口");
        assert!(client.instructions.iter().any(|instruction| matches!(
            instruction.instruction,
            Instruction::InvokeServerSegment { segment_id, .. } if segment_id == create_node_id
        )));
        assert!(
            client
                .instructions
                .iter()
                .any(|instruction| matches!(instruction.instruction, Instruction::Notify { .. }))
        );
        let server = image
            .server_functions
            .get(&create_node_id)
            .expect("服务端节点 segment");
        assert!(server.instructions.iter().any(|instruction| matches!(
            instruction.instruction,
            Instruction::CreateRecord { model_id: value } if value == model_id
        )));
        Ok(())
    }

    #[test]
    fn compiles_one_thousand_component_page_within_acceptance_budget() -> anyhow::Result<()> {
        let mut program = valid_program();
        program.pages[0].root.children = (0..1_000)
            .map(|index| ComponentNode {
                id: SymbolId::new(),
                component: "layout.section".to_owned(),
                state: DefinitionState::Known,
                properties: BTreeMap::new(),
                content: Some(crate::PropertyValue::text(format!("component-{index}"))),
                events: BTreeMap::new(),
                children: Vec::new(),
                style: ComponentStyle::default(),
            })
            .collect();
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let started = Instant::now();
        let image = ProgramCompiler::new("performance", &components, &capabilities).compile(
            &program,
            "revision-performance",
            ImageTarget::Universal,
        )?;
        assert_eq!(image.pages[&program.pages[0].id].root.children.len(), 1_000);
        assert!(started.elapsed() < Duration::from_secs(2));
        Ok(())
    }

    #[test]
    fn validates_ten_thousand_node_logic_graph_within_acceptance_budget() -> anyhow::Result<()> {
        let mut program = valid_program();
        let function_id = SymbolId::new();
        let node_ids = (0..10_000).map(|_| SymbolId::new()).collect::<Vec<_>>();
        let nodes = node_ids
            .iter()
            .enumerate()
            .map(|(index, id)| FunctionNode {
                id: *id,
                name: format!("math-{index}"),
                state: DefinitionState::Known,
                editor: FunctionNodeEditor::default(),
                kind: FunctionNodeKind::Math {
                    operator: crate::MathOperator::Add,
                },
            })
            .collect();
        let edges = node_ids
            .windows(2)
            .map(|pair| GraphEdge {
                id: SymbolId::new(),
                from_node: pair[0],
                from_port: "value".to_owned(),
                to_node: pair[1],
                to_port: "value".to_owned(),
            })
            .collect();
        program.functions.push(FunctionDefinition {
            id: function_id,
            name: "large-graph".to_owned(),
            title: "大逻辑图".to_owned(),
            state: DefinitionState::Known,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: FunctionGraph { nodes, edges },
            required_permissions: Vec::new(),
        });
        let components = test_catalog();
        let capabilities = CapabilityCatalog::default();
        let started = Instant::now();
        let image = ProgramCompiler::new("performance", &components, &capabilities).compile(
            &program,
            "revision-performance",
            ImageTarget::Universal,
        )?;
        assert_eq!(
            image.client_functions[&function_id].instructions.len(),
            10_000
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        Ok(())
    }
}
