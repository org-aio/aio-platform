use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BytecodeInstruction, BytecodeSegment, CapabilityCatalog, CompiledExpressionIndex,
    CompiledModel, CompiledPage, CompiledPageRenderer, CompiledRoute, CompiledTable, CompiledTree,
    DefinitionState, EffectKind, FieldDefinition, FunctionDefinition, FunctionNode,
    FunctionNodeKind, GraphEdge, ImageTarget, Instruction, ModelDefinition, PROGRAM_SCHEMA_VERSION,
    PageDefinition, PageRendererDefinition, ProgramDefinition, ProgramImage, SymbolId,
    TableDefinition, validate_route_path,
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
    capabilities: &'a CapabilityCatalog,
}

impl<'a> ProgramCompiler<'a> {
    #[must_use]
    pub fn new(compiler_version: &'a str, capabilities: &'a CapabilityCatalog) -> Self {
        Self {
            compiler_version,
            capabilities,
        }
    }

    pub fn compile(
        &self,
        definition: &ProgramDefinition,
        revision_id: impl Into<String>,
        target: ImageTarget,
    ) -> Result<ProgramImage, CompileFailure> {
        let mut diagnostics = Vec::new();
        self.validate_schema(definition, &mut diagnostics);
        let symbols = self.resolve_symbols(definition, &mut diagnostics);
        self.infer_types(definition, &mut diagnostics);
        self.link_pages_and_capabilities(definition, &symbols, &mut diagnostics);
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
        Ok(ProgramImage {
            schema_version: PROGRAM_SCHEMA_VERSION,
            compiler_version: self.compiler_version.to_owned(),
            content_hash,
            program_id: definition.id,
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
                validate_field_options(field, diagnostics);
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
            validate_page_references(page, &symbols, diagnostics);
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

    fn link_pages_and_capabilities(
        &self,
        definition: &ProgramDefinition,
        symbols: &BTreeSet<SymbolId>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        self.validate_page_metadata(definition, symbols, diagnostics);
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

    fn validate_page_metadata(
        &self,
        definition: &ProgramDefinition,
        _symbols: &BTreeSet<SymbolId>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for page in &definition.pages {
            validate_page_renderer(definition, page, diagnostics);
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
            dependencies.insert(page.id, page_model_dependencies(page));
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
                let field_titles = model
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(slot, field)| (slot as u32, field.title.clone()))
                    .collect::<BTreeMap<_, _>>();
                let field_options = model
                    .fields
                    .iter()
                    .filter_map(|field| {
                        field_slots
                            .get(&field.id)
                            .map(|slot| (*slot, field.options.clone()))
                    })
                    .collect::<BTreeMap<_, _>>();
                let required_fields = model
                    .fields
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, field)| field.required.then_some(slot as u32))
                    .collect::<Vec<_>>();
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
                        title: model.title.clone(),
                        field_slots,
                        field_types,
                        field_names,
                        field_titles,
                        field_options,
                        required_fields,
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
        BTreeMap<SymbolId, CompiledPage>,
        BTreeMap<SymbolId, BytecodeSegment>,
        BTreeMap<SymbolId, BytecodeSegment>,
        Vec<CompiledRoute>,
    ) {
        let pages = definition
            .pages
            .iter()
            .map(|page| (page.id, compile_page(definition, page)))
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
        pages: &BTreeMap<SymbolId, CompiledPage>,
        routes: &[CompiledRoute],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for route in routes {
            if !pages.contains_key(&route.page_id) {
                diagnostics.push(diagnostic(
                    "SMOKE_ROUTE_PAGE_MISSING",
                    CompilerStage::SmokeTest,
                    format!("路由 {} 的页面未进入编译产物", route.path),
                    Some(route.id),
                ));
            }
        }
    }
}

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
}

#[must_use]
pub fn compile_page(definition: &ProgramDefinition, page: &PageDefinition) -> CompiledPage {
    let renderer = match &page.renderer {
        PageRendererDefinition::ConventionFile => CompiledPageRenderer::ConventionFile {
            module_name: convention_page_module_name(&definition.name, &page.name),
            expected_path: convention_page_path(&definition.name, &page.name),
        },
        PageRendererDefinition::TreeTable { tree, table } => CompiledPageRenderer::TreeTable {
            tree: CompiledTree {
                model_id: tree.model_id.unwrap_or(page.id),
                label_field_id: tree.label_field_id.unwrap_or(page.id),
                parent_field_id: tree.parent_field_id,
                table_relation_field_id: tree.table_relation_field_id.unwrap_or(page.id),
            },
            table: compile_table(table, page.id),
        },
        PageRendererDefinition::CrudTable { table } => CompiledPageRenderer::CrudTable {
            table: compile_table(table, page.id),
        },
    };
    CompiledPage {
        id: page.id,
        name: page.name.clone(),
        title: page.title.clone(),
        renderer,
    }
}

fn compile_table(table: &TableDefinition, fallback: SymbolId) -> CompiledTable {
    CompiledTable {
        model_id: table.model_id.unwrap_or(fallback),
        page_size: table.page_size,
    }
}

#[must_use]
pub fn convention_page_module_name(program_name: &str, page_name: &str) -> String {
    let program_name = rust_module_segment(program_name);
    let page_name = rust_module_segment(page_name);
    format!("{program_name}__{page_name}")
}

#[must_use]
pub fn convention_page_path(program_name: &str, page_name: &str) -> String {
    let module_name = convention_page_module_name(program_name, page_name);
    format!("src/pages/{module_name}.rs")
}

fn rust_module_segment(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
        } else if !output.ends_with('_') {
            output.push('_');
        }
    }
    output.trim_matches('_').to_owned()
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
        PageRendererDefinition::ConventionFile => {}
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
    for reference in references {
        check_reference(reference, symbols, diagnostics, page.id);
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
        PageRendererDefinition::ConventionFile => {}
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
        PageRendererDefinition::ConventionFile => {}
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
            FunctionNodeKind::Notify { level } => Instruction::Notify {
                level: format!("{level:?}").to_lowercase(),
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
    use super::*;
    use crate::{
        DefinitionState, FieldDefinition, MenuActionAccess, MenuDefinition, MenuRowActions,
        ModelDefinition, PageRendererDefinition, RouteDefinition, TableDefinition, TreeDefinition,
        ValueType,
    };

    fn model(name: &str, title: &str) -> ModelDefinition {
        ModelDefinition {
            id: SymbolId::new(),
            name: name.to_owned(),
            title: title.to_owned(),
            state: DefinitionState::Known,
            fields: vec![
                FieldDefinition {
                    id: SymbolId::new(),
                    name: "name".to_owned(),
                    title: "名称".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions {
                        filterable: true,
                        ..crate::FieldOptions::default()
                    },
                    relation_model_id: None,
                },
                FieldDefinition {
                    id: SymbolId::new(),
                    name: "category_id".to_owned(),
                    title: "分类".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation_model_id: None,
                },
            ],
            indexes: Vec::new(),
        }
    }

    fn crud_program() -> ProgramDefinition {
        let mut program = ProgramDefinition::empty("inventory", "资产");
        let model = model("asset", "资产");
        let page_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "asset_list".to_owned(),
            title: "资产列表".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::CrudTable {
                table: TableDefinition {
                    model_id: Some(model.id),
                    page_size: 20,
                },
            },
        });
        program.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: "asset_list".to_owned(),
            path: "/assets".to_owned(),
            page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });
        program.menus.push(MenuDefinition {
            id: SymbolId::new(),
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            icon: None,
            page_id: Some(page_id),
            enabled: true,
            children: Vec::new(),
            required_permissions: Vec::new(),
            row_actions: MenuRowActions {
                detail: MenuActionAccess::Public,
                edit: MenuActionAccess::Public,
                delete: MenuActionAccess::Hidden,
            },
        });
        program.models.push(model);
        program
    }

    #[test]
    fn compiles_crud_page_from_model_metadata() -> anyhow::Result<()> {
        let program = crud_program();
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        let page = &image.pages[&program.pages[0].id];
        assert!(matches!(
            page.renderer,
            CompiledPageRenderer::CrudTable { .. }
        ));
        assert_eq!(image.models[&program.models[0].id].title, "资产");
        Ok(())
    }

    #[test]
    fn rejects_builtin_page_without_model() {
        let mut program = crud_program();
        program.pages[0].renderer = PageRendererDefinition::CrudTable {
            table: TableDefinition::default(),
        };
        let error = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .expect_err("缺少模型必须被拒绝");
        assert!(
            error
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "TABLE_MODEL_REQUIRED")
        );
    }

    #[test]
    fn compiles_tree_table_with_explicit_relation() -> anyhow::Result<()> {
        let mut program = crud_program();
        let tree_model = model("category", "分类");
        let table_model = program.models[0].clone();
        program.pages[0].renderer = PageRendererDefinition::TreeTable {
            tree: TreeDefinition {
                model_id: Some(tree_model.id),
                label_field_id: Some(tree_model.fields[0].id),
                parent_field_id: Some(tree_model.fields[1].id),
                table_relation_field_id: Some(table_model.fields[1].id),
            },
            table: TableDefinition {
                model_id: Some(table_model.id),
                page_size: 20,
            },
        };
        program.models.push(tree_model);
        ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        Ok(())
    }

    #[test]
    fn content_hash_is_deterministic() -> anyhow::Result<()> {
        let program = crud_program();
        assert_eq!(content_hash(&program)?, content_hash(&program)?);
        Ok(())
    }

    #[test]
    fn convention_path_is_a_safe_rust_module_path() {
        assert_eq!(
            convention_page_module_name("aio-first-party", "API Keys"),
            "aio_first_party__api_keys"
        );
        assert_eq!(
            convention_page_path("aio-first-party", "API Keys"),
            "src/pages/aio_first_party__api_keys.rs"
        );
    }
}
