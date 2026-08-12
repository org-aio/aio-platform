use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BytecodeInstruction, BytecodeSegment, CapabilityCatalog, CompiledEndpointInput,
    CompiledEndpointOutput, CompiledExpressionIndex, CompiledModel, CompiledPage,
    CompiledPageEndpoint, CompiledPageRenderer, CompiledRoute, CompiledTable, CompiledTree,
    CrudTablePageProvider, DefinitionState, EffectKind, EndpointInputLocation, FieldDefinition,
    FunctionDefinition, FunctionNode, FunctionNodeKind, GraphEdge, ImageTarget, Instruction,
    ModelDefinition, PROGRAM_SCHEMA_VERSION, PageDefinition, PageEndpointSource,
    PageRendererDefinition, ProgramDefinition, ProgramImage, ProgramMenuTreePageProvider,
    RestFormPageProvider, RestMethod, RudiRouteInstruction, SymbolId, TableDefinition,
    TreeTablePageProvider, data_identifier_is_valid, endpoint_identifier_is_valid,
    function_nodes_can_connect, page_identifier_is_valid, page_provider_key,
    permission_identifier_is_valid, validate_route_path,
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
        )?;
        for diagnostic in &self.diagnostics {
            write!(
                formatter,
                "\n[{}][{:?}][{:?}] {}",
                diagnostic.code, diagnostic.stage, diagnostic.severity, diagnostic.message
            )?;
            if let Some(symbol_id) = diagnostic.symbol_id {
                write!(formatter, " ({symbol_id})")?;
            }
        }
        Ok(())
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
        let mut permission_names = BTreeSet::new();
        for permission in &definition.permissions {
            if !permission_identifier_is_valid(&permission.name) {
                diagnostics.push(diagnostic(
                    "PERMISSION_IDENTIFIER_INVALID",
                    CompilerStage::Schema,
                    format!("权限标识必须采用 领域:动作 格式: {}", permission.name),
                    Some(permission.id),
                ));
            }
            if permission.title.trim().is_empty() {
                diagnostics.push(diagnostic(
                    "PERMISSION_TITLE_EMPTY",
                    CompilerStage::Schema,
                    "权限标题不能为空",
                    Some(permission.id),
                ));
            }
            if !permission_names.insert(permission.name.as_str()) {
                diagnostics.push(diagnostic(
                    "PERMISSION_IDENTIFIER_DUPLICATE",
                    CompilerStage::Schema,
                    format!("权限标识重复: {}", permission.name),
                    Some(permission.id),
                ));
            }
            let mut effects = BTreeSet::new();
            for effect in &permission.allowed_effects {
                if !effects.insert(*effect) {
                    diagnostics.push(diagnostic(
                        "PERMISSION_EFFECT_DUPLICATE",
                        CompilerStage::Schema,
                        format!("权限 {} 重复声明 {:?} Effect", permission.name, effect),
                        Some(permission.id),
                    ));
                }
            }
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
            if !data_identifier_is_valid(&model.name) {
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
                if field.name == "id" {
                    diagnostics.push(diagnostic(
                        "FIELD_IDENTIFIER_RESERVED",
                        CompilerStage::Schema,
                        format!("模型 {} 的 id 由系统主键定义维护", model.name),
                        Some(field.id),
                    ));
                }
                if !data_identifier_is_valid(&field.name) {
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
            validate_model_indexes(model, diagnostics);
            validate_model_queries(definition, model, diagnostics);
            validate_model_validations(model, diagnostics);
            validate_model_audit(model, diagnostics);
        }
        validate_model_relations(definition, diagnostics);
        let mut function_names = BTreeSet::new();
        for function in &definition.functions {
            if !data_identifier_is_valid(&function.name) {
                diagnostics.push(diagnostic(
                    "FUNCTION_IDENTIFIER_INVALID",
                    CompilerStage::Schema,
                    format!("函数标识必须是 snake_case: {}", function.name),
                    Some(function.id),
                ));
            }
            if function.title.trim().is_empty() {
                diagnostics.push(diagnostic(
                    "FUNCTION_TITLE_EMPTY",
                    CompilerStage::Schema,
                    "函数标题不能为空",
                    Some(function.id),
                ));
            }
            if !function_names.insert(function.name.as_str()) {
                diagnostics.push(diagnostic(
                    "FUNCTION_IDENTIFIER_DUPLICATE",
                    CompilerStage::Schema,
                    format!("函数标识重复: {}", function.name),
                    Some(function.id),
                ));
            }
        }
        let routed_page_ids = definition
            .routes
            .iter()
            .map(|route| route.page_id)
            .collect::<BTreeSet<_>>();
        let mut page_names = BTreeSet::new();
        for page in &definition.pages {
            if !page_identifier_is_valid(&page.name) {
                diagnostics.push(diagnostic(
                    "PAGE_IDENTIFIER_INVALID",
                    CompilerStage::Schema,
                    format!(
                        "页面标识必须使用小写字母、数字、下划线或连字符: {}",
                        page.name
                    ),
                    Some(page.id),
                ));
            }
            if page.title.trim().is_empty() {
                diagnostics.push(diagnostic(
                    "PAGE_TITLE_EMPTY",
                    CompilerStage::Schema,
                    "页面标题不能为空",
                    Some(page.id),
                ));
            }
            if !page_names.insert(page.name.as_str()) {
                diagnostics.push(diagnostic(
                    "PAGE_IDENTIFIER_DUPLICATE",
                    CompilerStage::Schema,
                    format!("页面标识重复: {}", page.name),
                    Some(page.id),
                ));
            }
            if !routed_page_ids.contains(&page.id) {
                diagnostics.push(diagnostic(
                    "PAGE_ROUTE_MISSING",
                    CompilerStage::Schema,
                    format!("页面缺少可访问路由: {}", page.name),
                    Some(page.id),
                ));
            }
            validate_page_endpoints(page, diagnostics);
        }
        validate_global_endpoint_routes(definition, diagnostics);
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
            for query in &model.queries {
                insert_symbol(&mut symbols, query.id, diagnostics);
            }
            for validation in &model.validations {
                insert_symbol(&mut symbols, validation.id, diagnostics);
            }
        }
        for page in &definition.pages {
            insert_symbol(&mut symbols, page.id, diagnostics);
            check_state(page.id, &page.state, diagnostics);
            for endpoint in &page.endpoints {
                insert_symbol(&mut symbols, endpoint.id, diagnostics);
                check_state(endpoint.id, &endpoint.state, diagnostics);
                for input in &endpoint.inputs {
                    insert_symbol(&mut symbols, input.id, diagnostics);
                }
                for output in &endpoint.outputs {
                    insert_symbol(&mut symbols, output.id, diagnostics);
                }
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

        for model in &definition.models {
            for field in &model.fields {
                validate_value_type_references(&field.value_type, &symbols, diagnostics, field.id);
            }
        }
        for menu in &definition.menus {
            validate_menu_references(menu, &symbols, diagnostics);
        }
        for page in &definition.pages {
            validate_page_references(page, &symbols, diagnostics);
        }
        for route in &definition.routes {
            check_reference(route.page_id, &symbols, diagnostics, route.id);
            for permission_id in &route.required_permissions {
                check_reference(*permission_id, &symbols, diagnostics, route.id);
            }
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
                if !function_nodes_can_connect(&from.kind, &to.kind) {
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
                let field_relations = model
                    .fields
                    .iter()
                    .filter_map(|field| {
                        field.relation.clone().and_then(|relation| {
                            field_slots.get(&field.id).map(|slot| (*slot, relation))
                        })
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
                            unique: index.unique,
                        }
                    })
                    .collect();
                (
                    model.id,
                    CompiledModel {
                        id: model.id,
                        name: model.name.clone(),
                        title: model.title.clone(),
                        primary_key: model.primary_key,
                        field_slots,
                        field_types,
                        field_names,
                        field_titles,
                        field_options,
                        field_relations,
                        required_fields,
                        expression_indexes,
                        audit: model.audit.clone(),
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

#[must_use]
pub fn compile_page(definition: &ProgramDefinition, page: &PageDefinition) -> CompiledPage {
    let renderer = match &page.renderer {
        PageRendererDefinition::ConventionFile => CompiledPageRenderer::ConventionFile {
            module_name: convention_page_module_name(&definition.name, &page.name),
            expected_path: convention_page_path(&definition.name, &page.name),
        },
        PageRendererDefinition::Extension {
            extension_type,
            schema_version,
            config,
        } => CompiledPageRenderer::Extension {
            extension_type: extension_type.clone(),
            schema_version: *schema_version,
            config: config.clone(),
        },
        PageRendererDefinition::MenuTree => CompiledPageRenderer::MenuTree {
            provider_key: page_provider_key::<ProgramMenuTreePageProvider>(),
        },
        PageRendererDefinition::TreeTable { tree, table } => CompiledPageRenderer::TreeTable {
            provider_key: page_provider_key::<TreeTablePageProvider>(),
            tree: CompiledTree {
                model_id: tree.model_id.unwrap_or(page.id),
                label_field_id: tree.label_field_id.unwrap_or(page.id),
                parent_field_id: tree.parent_field_id,
                table_relation_field_id: tree.table_relation_field_id.unwrap_or(page.id),
            },
            table: compile_table(table, page.id),
        },
        PageRendererDefinition::CrudTable { table } => CompiledPageRenderer::CrudTable {
            provider_key: page_provider_key::<CrudTablePageProvider>(),
            table: compile_table(table, page.id),
        },
    };
    let endpoints = compile_page_endpoints(definition, page);
    CompiledPage {
        id: page.id,
        name: page.name.clone(),
        title: page.title.clone(),
        renderer,
        endpoints,
    }
}

fn compile_page_endpoints(
    definition: &ProgramDefinition,
    page: &PageDefinition,
) -> Vec<CompiledPageEndpoint> {
    let mut endpoints = built_in_page_endpoints(definition, page);
    let provider_key = page_provider_key::<RestFormPageProvider>();
    endpoints.extend(
        page.endpoints
            .iter()
            .filter(|endpoint| endpoint.state.is_known())
            .map(|endpoint| CompiledPageEndpoint {
                id: endpoint.id.to_string(),
                title: endpoint.display_title(),
                description: endpoint.description.clone(),
                method: endpoint.method,
                path: endpoint.path.clone(),
                inputs: endpoint
                    .inputs
                    .iter()
                    .map(|input| CompiledEndpointInput {
                        name: input.name.clone(),
                        title: input.title.clone(),
                        location: input.location,
                        value_type: input.value_type.clone(),
                        required: input.required,
                    })
                    .collect(),
                outputs: endpoint
                    .outputs
                    .iter()
                    .map(|output| CompiledEndpointOutput {
                        name: output.name.clone(),
                        title: output.title.clone(),
                        value_type: output.value_type.clone(),
                    })
                    .collect(),
                source: match endpoint.implementation {
                    crate::EndpointImplementationDefinition::Native { .. } => {
                        PageEndpointSource::Native
                    }
                    crate::EndpointImplementationDefinition::Convention => {
                        PageEndpointSource::Convention
                    }
                },
                route_instruction: RudiRouteInstruction {
                    provider_key: provider_key.clone(),
                },
            }),
    );
    endpoints
}

fn built_in_page_endpoints(
    definition: &ProgramDefinition,
    page: &PageDefinition,
) -> Vec<CompiledPageEndpoint> {
    let (table, provider_key) = match &page.renderer {
        PageRendererDefinition::CrudTable { table } => {
            (table, page_provider_key::<CrudTablePageProvider>())
        }
        PageRendererDefinition::TreeTable { table, .. } => {
            (table, page_provider_key::<TreeTablePageProvider>())
        }
        PageRendererDefinition::ConventionFile
        | PageRendererDefinition::Extension { .. }
        | PageRendererDefinition::MenuTree => {
            return Vec::new();
        }
    };
    let Some(model_id) = table.model_id else {
        return Vec::new();
    };
    let Some(model) = definition.models.iter().find(|model| model.id == model_id) else {
        return Vec::new();
    };
    let records_path = format!("/api/runtime/models/{model_id}/records");
    let body_inputs = model
        .fields
        .iter()
        .filter(|field| field.options.form_visible)
        .map(|field| CompiledEndpointInput {
            name: field.name.clone(),
            title: field.title.clone(),
            location: EndpointInputLocation::Body,
            value_type: field.value_type.clone(),
            required: field.required,
        })
        .collect::<Vec<_>>();
    let model_outputs = model
        .fields
        .iter()
        .filter(|field| field.options.detail_visible)
        .map(|field| CompiledEndpointOutput {
            name: field.name.clone(),
            title: field.title.clone(),
            value_type: field.value_type.clone(),
        })
        .collect::<Vec<_>>();
    let page_inputs = vec![
        CompiledEndpointInput {
            name: "o".to_owned(),
            title: "偏移量".to_owned(),
            location: EndpointInputLocation::Query,
            value_type: crate::ValueType::Integer,
            required: false,
        },
        CompiledEndpointInput {
            name: "s".to_owned(),
            title: "每页条数".to_owned(),
            location: EndpointInputLocation::Query,
            value_type: crate::ValueType::Integer,
            required: false,
        },
    ];
    let record_id = CompiledEndpointInput {
        name: "record_id".to_owned(),
        title: format!("{} ID", model.title),
        location: EndpointInputLocation::Path,
        value_type: crate::ValueType::Text,
        required: true,
    };
    let build = |name: &str,
                 title: String,
                 method: RestMethod,
                 path: String,
                 inputs: Vec<CompiledEndpointInput>,
                 outputs: Vec<CompiledEndpointOutput>| CompiledPageEndpoint {
        id: format!("builtin:{}:{name}", page.id),
        description: format!("由 {} 模型元数据推导", model.title),
        title,
        method,
        path,
        inputs,
        outputs,
        source: PageEndpointSource::BuiltIn,
        route_instruction: RudiRouteInstruction {
            provider_key: provider_key.clone(),
        },
    };
    vec![
        build(
            "query",
            format!("查询{}", model.title),
            RestMethod::Get,
            records_path.clone(),
            page_inputs,
            vec![
                CompiledEndpointOutput {
                    name: "d".to_owned(),
                    title: "记录列表".to_owned(),
                    value_type: crate::ValueType::List {
                        item: Box::new(crate::ValueType::Object { model_id }),
                    },
                },
                CompiledEndpointOutput {
                    name: "t".to_owned(),
                    title: "总数".to_owned(),
                    value_type: crate::ValueType::Integer,
                },
            ],
        ),
        build(
            "create",
            format!("新增{}", model.title),
            RestMethod::Post,
            records_path.clone(),
            body_inputs.clone(),
            model_outputs.clone(),
        ),
        build(
            "update",
            format!("修改{}", model.title),
            RestMethod::Patch,
            format!("{records_path}/{{record_id}}"),
            std::iter::once(record_id.clone())
                .chain(body_inputs)
                .collect(),
            model_outputs,
        ),
        build(
            "delete",
            format!("删除{}", model.title),
            RestMethod::Delete,
            format!("{records_path}/{{record_id}}"),
            vec![record_id],
            Vec::new(),
        ),
        build(
            "import",
            format!("导入{}", model.title),
            RestMethod::Post,
            format!("{records_path}/import"),
            vec![CompiledEndpointInput {
                name: "file".to_owned(),
                title: "导入文件".to_owned(),
                location: EndpointInputLocation::Body,
                value_type: crate::ValueType::File,
                required: true,
            }],
            vec![CompiledEndpointOutput {
                name: "created".to_owned(),
                title: "导入数量".to_owned(),
                value_type: crate::ValueType::Integer,
            }],
        ),
        build(
            "export",
            format!("导出{}", model.title),
            RestMethod::Get,
            format!("{records_path}/export"),
            Vec::new(),
            vec![CompiledEndpointOutput {
                name: "file".to_owned(),
                title: "导出文件".to_owned(),
                value_type: crate::ValueType::File,
            }],
        ),
    ]
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
        PageRendererDefinition::ConventionFile
        | PageRendererDefinition::Extension { .. }
        | PageRendererDefinition::MenuTree => {}
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
        PageRendererDefinition::ConventionFile
        | PageRendererDefinition::Extension { .. }
        | PageRendererDefinition::MenuTree => {}
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
        PageRendererDefinition::ConventionFile
        | PageRendererDefinition::Extension { .. }
        | PageRendererDefinition::MenuTree => {}
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
        DefinitionState, EndpointInputDefinition, EndpointInputLocation, EndpointOutputDefinition,
        FieldDefinition, FunctionGraph, FunctionNodeEditor, MenuActionAccess, MenuDefinition,
        MenuRowActions, ModelDefinition, PageEndpointDefinition, PageRendererDefinition,
        PermissionDefinition, PortDefinition, RestMethod, RouteDefinition, TableDefinition,
        TreeDefinition, ValueType,
    };

    #[test]
    fn compile_failure_display_includes_actionable_diagnostics() {
        let symbol_id = SymbolId::new();
        let failure = CompileFailure {
            diagnostics: vec![diagnostic(
                "PAGE_ENDPOINT_INPUT_INVALID",
                CompilerStage::Schema,
                "接口入参标识无效",
                Some(symbol_id),
            )],
        };

        let message = failure.to_string();
        assert!(message.contains("PAGE_ENDPOINT_INPUT_INVALID"));
        assert!(message.contains("接口入参标识无效"));
        assert!(message.contains(&symbol_id.to_string()));
    }

    fn model(name: &str, title: &str) -> ModelDefinition {
        ModelDefinition {
            id: SymbolId::new(),
            name: name.to_owned(),
            title: title.to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
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
                    relation: None,
                },
                FieldDefinition {
                    id: SymbolId::new(),
                    name: "category_id".to_owned(),
                    title: "分类".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
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
            endpoints: Vec::new(),
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
        assert_eq!(page.endpoints.len(), 6);
        let model_id = program.models[0].id;
        assert_eq!(
            page.endpoints[0].path,
            format!("/api/runtime/models/{model_id}/records")
        );
        assert_eq!(
            page.endpoints[4].path,
            format!("/api/runtime/models/{model_id}/records/import")
        );
        assert_eq!(
            page.endpoints[0].route_instruction.provider_key,
            page_provider_key::<CrudTablePageProvider>()
        );
        assert_eq!(image.models[&program.models[0].id].title, "资产");
        Ok(())
    }

    #[test]
    fn compiles_program_menu_tree_without_record_endpoints() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("admin", "管理后台");
        let page_id = SymbolId::new();
        program.pages.push(PageDefinition {
            id: page_id,
            name: "menus".to_owned(),
            title: "菜单挂载".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::MenuTree,
            endpoints: Vec::new(),
        });
        program.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: "menus".to_owned(),
            path: "/menus".to_owned(),
            page_id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });

        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-menu",
            ImageTarget::Universal,
        )?;
        let page = &image.pages[&page_id];

        assert!(matches!(
            &page.renderer,
            CompiledPageRenderer::MenuTree { provider_key }
                if provider_key == &page_provider_key::<ProgramMenuTreePageProvider>()
        ));
        assert!(page.endpoints.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_invalid_or_duplicate_permission_identifiers() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.permissions = vec![
            PermissionDefinition {
                id: SymbolId::new(),
                name: "asset.read".to_owned(),
                title: "查看资产".to_owned(),
                allowed_effects: Vec::new(),
            },
            PermissionDefinition {
                id: SymbolId::new(),
                name: "asset.read".to_owned(),
                title: "重复权限".to_owned(),
                allowed_effects: Vec::new(),
            },
        ];
        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无效权限定义不应通过编译"))?;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PERMISSION_IDENTIFIER_INVALID")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PERMISSION_IDENTIFIER_DUPLICATE")
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_or_duplicate_page_identifiers() -> anyhow::Result<()> {
        let mut program = crud_program();
        let mut duplicate = program.pages[0].clone();
        duplicate.id = SymbolId::new();
        let mut invalid = program.pages[0].clone();
        invalid.id = SymbolId::new();
        invalid.name = "Asset Page".to_owned();
        invalid.title.clear();
        program.pages.extend([duplicate, invalid]);

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无效页面定义不应通过编译"))?;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_IDENTIFIER_INVALID")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_IDENTIFIER_DUPLICATE")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_TITLE_EMPTY")
        );
        Ok(())
    }

    #[test]
    fn rejects_page_without_route() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.routes.clear();

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无路由页面不应通过编译"))?;
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_ROUTE_MISSING")
        );
        Ok(())
    }

    #[test]
    fn rejects_invalid_duplicate_or_untitled_functions() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.functions = vec![
            FunctionDefinition {
                id: SymbolId::new(),
                name: "Load Asset".to_owned(),
                title: String::new(),
                state: DefinitionState::Known,
                inputs: Vec::new(),
                outputs: Vec::new(),
                graph: FunctionGraph::default(),
                required_permissions: Vec::new(),
            },
            FunctionDefinition {
                id: SymbolId::new(),
                name: "Load Asset".to_owned(),
                title: "重复函数".to_owned(),
                state: DefinitionState::Known,
                inputs: Vec::new(),
                outputs: Vec::new(),
                graph: FunctionGraph::default(),
                required_permissions: Vec::new(),
            },
        ];

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("无效函数定义不应通过编译"))?;

        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "FUNCTION_IDENTIFIER_INVALID")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "FUNCTION_TITLE_EMPTY")
        );
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "FUNCTION_IDENTIFIER_DUPLICATE")
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_model_and_field_references_in_structured_types() -> anyhow::Result<()> {
        let mut program = crud_program();
        let missing_model_id = SymbolId::new();
        let missing_field_id = SymbolId::new();
        let field_owner_id = program.models[0].fields[0].id;
        program.models[0].fields[0].value_type = ValueType::Optional {
            value: Box::new(ValueType::Object {
                model_id: missing_model_id,
            }),
        };
        let port_id = SymbolId::new();
        let node_id = SymbolId::new();
        let input_node_id = SymbolId::new();
        let object_node_id = SymbolId::new();
        let list_node_id = SymbolId::new();
        let format_node_id = SymbolId::new();
        let validation_node_id = SymbolId::new();
        program.functions.push(FunctionDefinition {
            id: SymbolId::new(),
            name: "load_asset".to_owned(),
            title: "读取资产".to_owned(),
            state: DefinitionState::Known,
            inputs: vec![PortDefinition {
                id: port_id,
                name: "asset".to_owned(),
                value_type: ValueType::Object {
                    model_id: missing_model_id,
                },
            }],
            outputs: Vec::new(),
            graph: FunctionGraph {
                nodes: vec![
                    FunctionNode {
                        id: node_id,
                        name: "read_missing_field".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::FieldAccess {
                            object: SymbolId::new(),
                            field_id: missing_field_id,
                        },
                    },
                    FunctionNode {
                        id: input_node_id,
                        name: "missing_input".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Input {
                            port_id: SymbolId::new(),
                        },
                    },
                    FunctionNode {
                        id: object_node_id,
                        name: "object_with_missing_references".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Object {
                            fields: BTreeMap::from([(missing_field_id, SymbolId::new())]),
                        },
                    },
                    FunctionNode {
                        id: list_node_id,
                        name: "list_with_missing_item".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::List {
                            items: vec![SymbolId::new()],
                        },
                    },
                    FunctionNode {
                        id: format_node_id,
                        name: "format_with_missing_value".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::Format {
                            template: "{0}".to_owned(),
                            values: vec![SymbolId::new()],
                        },
                    },
                    FunctionNode {
                        id: validation_node_id,
                        name: "validation_with_missing_field".to_owned(),
                        state: DefinitionState::Known,
                        editor: FunctionNodeEditor::default(),
                        kind: FunctionNodeKind::ValidateForm {
                            rules: vec![crate::ValidationRule {
                                field_id: missing_field_id,
                                rule: crate::ValidationRuleKind::Required,
                                message: "不能为空".to_owned(),
                            }],
                        },
                    },
                ],
                edges: Vec::new(),
            },
            required_permissions: Vec::new(),
        });

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err()
            .ok_or_else(|| anyhow::anyhow!("悬空模型与字段引用不应通过编译"))?;
        let unresolved_owners = failure
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SYMBOL_UNRESOLVED")
            .filter_map(|diagnostic| diagnostic.symbol_id)
            .collect::<BTreeSet<_>>();

        assert!(unresolved_owners.contains(&field_owner_id));
        assert!(unresolved_owners.contains(&port_id));
        assert!(unresolved_owners.contains(&node_id));
        assert!(unresolved_owners.contains(&input_node_id));
        assert!(unresolved_owners.contains(&object_node_id));
        assert!(unresolved_owners.contains(&list_node_id));
        assert!(unresolved_owners.contains(&format_node_id));
        assert!(unresolved_owners.contains(&validation_node_id));
        Ok(())
    }

    #[test]
    fn compiles_composable_model_audit_metadata() -> anyhow::Result<()> {
        let tenant_id = SymbolId::new();
        let deleted_id = SymbolId::new();
        let model_id = SymbolId::new();
        let model = ModelDefinition {
            id: model_id,
            name: "asset".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: tenant_id,
                    name: "tenant_id".to_owned(),
                    title: "租户".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: deleted_id,
                    name: "deleted".to_owned(),
                    title: "逻辑删除".to_owned(),
                    value_type: ValueType::Boolean,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition {
                fields: vec![
                    crate::ModelAuditField {
                        kind: crate::AuditFieldKind::TenantId,
                        field_id: tenant_id,
                    },
                    crate::ModelAuditField {
                        kind: crate::AuditFieldKind::Deleted,
                        field_id: deleted_id,
                    },
                ],
            },
        };
        let mut program = ProgramDefinition::empty("inventory", "资产");
        program.models.push(model);
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-audit",
            ImageTarget::Universal,
        )?;
        assert_eq!(image.models[&model_id].audit.fields.len(), 2);
        Ok(())
    }

    #[test]
    fn compiles_bidirectional_relationship_query_and_validation_metadata() -> anyhow::Result<()> {
        let mut program = ProgramDefinition::empty("directory", "组织目录");
        let department_id = SymbolId::new();
        let department_name_id = SymbolId::new();
        let department_users_id = SymbolId::new();
        let user_id = SymbolId::new();
        let user_name_id = SymbolId::new();
        let user_department_id = SymbolId::new();
        let user_phone_id = SymbolId::new();
        let user_email_id = SymbolId::new();
        program.models.push(ModelDefinition {
            id: department_id,
            name: "department".to_owned(),
            title: "部门".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: department_name_id,
                    name: "name".to_owned(),
                    title: "部门名称".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: department_users_id,
                    name: "users".to_owned(),
                    title: "用户".to_owned(),
                    value_type: ValueType::List {
                        item: Box::new(ValueType::Object { model_id: user_id }),
                    },
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions {
                        validation: crate::FieldValidation {
                            unique_items: true,
                            ..crate::FieldValidation::default()
                        },
                        ..crate::FieldOptions::default()
                    },
                    relation: Some(crate::FieldRelation {
                        kind: crate::RelationKind::OneToMany,
                        target_model_id: user_id,
                        target_field_id: user_department_id,
                    }),
                },
            ],
            indexes: vec![crate::ModelIndexDefinition {
                id: SymbolId::new(),
                fields: vec![department_name_id],
                unique: false,
            }],
            queries: vec![crate::ModelQueryDefinition {
                id: SymbolId::new(),
                name: "search_by_name".to_owned(),
                title: "按部门和用户名查询".to_owned(),
                conjunction: crate::QueryConjunction::All,
                conditions: vec![
                    crate::QueryCondition::Field {
                        field_id: department_name_id,
                        operator: crate::QueryOperator::Contains,
                        parameter: "department_name".to_owned(),
                    },
                    crate::QueryCondition::Relation {
                        relation_field_id: department_users_id,
                        target_field_id: user_name_id,
                        operator: crate::QueryOperator::Contains,
                        parameter: "user_name".to_owned(),
                    },
                ],
            }],
            validations: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
        });
        program.models.push(ModelDefinition {
            id: user_id,
            name: "user".to_owned(),
            title: "用户".to_owned(),
            state: DefinitionState::Known,
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            fields: vec![
                FieldDefinition {
                    id: user_name_id,
                    name: "name".to_owned(),
                    title: "用户名".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: user_department_id,
                    name: "department".to_owned(),
                    title: "部门".to_owned(),
                    value_type: ValueType::Object {
                        model_id: department_id,
                    },
                    state: DefinitionState::Known,
                    required: true,
                    options: crate::FieldOptions::default(),
                    relation: Some(crate::FieldRelation {
                        kind: crate::RelationKind::ManyToOne,
                        target_model_id: department_id,
                        target_field_id: department_users_id,
                    }),
                },
                FieldDefinition {
                    id: user_phone_id,
                    name: "phone".to_owned(),
                    title: "手机号".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
                FieldDefinition {
                    id: user_email_id,
                    name: "email".to_owned(),
                    title: "邮箱".to_owned(),
                    value_type: ValueType::Text,
                    state: DefinitionState::Known,
                    required: false,
                    options: crate::FieldOptions::default(),
                    relation: None,
                },
            ],
            indexes: Vec::new(),
            queries: Vec::new(),
            validations: vec![crate::ModelValidationDefinition {
                id: SymbolId::new(),
                message: "填写手机号时必须填写邮箱".to_owned(),
                rule: crate::ModelValidationRule::RequiredWhenPresent {
                    field_id: user_email_id,
                    when_field_id: user_phone_id,
                },
            }],
            audit: crate::ModelAuditDefinition::default(),
        });
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-relationship",
            ImageTarget::Universal,
        )?;
        assert_eq!(image.models.len(), 2);
        let user_model = &image.models[&user_id];
        let department_slot = user_model.field_slots[&user_department_id];
        assert_eq!(
            user_model.field_relations[&department_slot].kind,
            crate::RelationKind::ManyToOne
        );
        assert_eq!(
            user_model.field_relations[&department_slot].target_model_id,
            department_id
        );
        Ok(())
    }

    #[test]
    fn compiles_custom_rest_endpoint_with_rudi_form_instruction() -> anyhow::Result<()> {
        let mut program = crud_program();
        let endpoint_id = SymbolId::new();
        program.pages[0].endpoints.push(PageEndpointDefinition {
            id: endpoint_id,
            title: "批量停用资产".to_owned(),
            description: "批量停用指定分类中的资产".to_owned(),
            state: DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Convention,
            method: RestMethod::Post,
            path: "/api/assets/{categoryId}/batch-disable".to_owned(),
            inputs: vec![EndpointInputDefinition {
                id: SymbolId::new(),
                name: "categoryId".to_owned(),
                title: "分类 ID".to_owned(),
                location: EndpointInputLocation::Path,
                value_type: ValueType::Text,
                required: true,
            }],
            outputs: vec![EndpointOutputDefinition {
                id: SymbolId::new(),
                name: "updatedCount".to_owned(),
                title: "停用数量".to_owned(),
                value_type: ValueType::Integer,
            }],
        });

        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
        let endpoint = image.pages[&program.pages[0].id]
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id.to_string())
            .ok_or_else(|| anyhow::anyhow!("自定义接口未进入编译产物"))?;
        assert_eq!(endpoint.method, RestMethod::Post);
        assert_eq!(endpoint.title, "批量停用资产");
        assert_eq!(endpoint.source, PageEndpointSource::Convention);
        assert_eq!(
            endpoint.route_instruction.provider_key,
            page_provider_key::<RestFormPageProvider>()
        );
        Ok(())
    }

    #[test]
    fn rejects_declared_route_that_conflicts_with_builtin_endpoint() {
        let mut program = crud_program();
        let model_id = program.models[0].id;
        program.pages[0].endpoints.push(PageEndpointDefinition {
            id: SymbolId::new(),
            title: "重复查询".to_owned(),
            description: "错误覆盖内置查询".to_owned(),
            state: DefinitionState::Known,
            implementation: crate::EndpointImplementationDefinition::Convention,
            method: RestMethod::Get,
            path: format!("/api/runtime/models/{model_id}/records"),
            inputs: Vec::new(),
            outputs: Vec::new(),
        });

        let failure = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&program, "revision-1", ImageTarget::Universal)
            .err();

        assert!(failure.is_some_and(|failure| {
            failure
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PAGE_ENDPOINT_ROUTE_DUPLICATE_GLOBAL")
        }));
    }

    #[test]
    fn allows_multiple_pages_to_consume_the_same_builtin_model_routes() -> anyhow::Result<()> {
        let mut program = crud_program();
        let mut second_page = program.pages[0].clone();
        second_page.id = SymbolId::new();
        second_page.name = "asset_overview".to_owned();
        second_page.title = "资产概览".to_owned();
        program.routes.push(RouteDefinition {
            id: SymbolId::new(),
            name: second_page.name.clone(),
            path: "/asset-overview".to_owned(),
            page_id: second_page.id,
            state: DefinitionState::Known,
            required_permissions: Vec::new(),
        });
        program.pages.push(second_page);

        ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;
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
    fn compiles_consumer_extension_without_interpreting_its_config() -> anyhow::Result<()> {
        let mut program = crud_program();
        program.pages[0].renderer = PageRendererDefinition::Extension {
            extension_type: "aio::pages::AuditPage".to_owned(),
            schema_version: 3,
            config: serde_json::json!({"resource_id": "audit"}),
        };

        let image = ProgramCompiler::new("test", &CapabilityCatalog::default()).compile(
            &program,
            "revision-1",
            ImageTarget::Universal,
        )?;

        assert!(matches!(
            image.pages.get(&program.pages[0].id).map(|page| &page.renderer),
            Some(CompiledPageRenderer::Extension {
                extension_type,
                schema_version: 3,
                config,
            }) if extension_type == "aio::pages::AuditPage"
                && config == &serde_json::json!({"resource_id": "audit"})
        ));
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
