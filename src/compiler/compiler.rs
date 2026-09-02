use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BytecodeInstruction, BytecodeSegment, CapabilityCatalog, CompiledEndpointInput,
    CompiledEndpointOutput, CompiledExpressionIndex, CompiledModel, CompiledPage,
    CompiledPageEndpoint, CompiledPageRenderer, CompiledRoute, CompiledTable, CompiledTree,
    DefinitionState, EffectKind, EndpointInputLocation, FieldDefinition, FunctionDefinition,
    FunctionNode, FunctionNodeKind, GraphEdge, ImageTarget, Instruction, ModelDefinition,
    PROGRAM_SCHEMA_VERSION, PageDefinition, PageEndpointSource, PageRendererDefinition,
    ProgramDefinition, ProgramImage, RestMethod, SymbolId, TableDefinition,
    data_identifier_is_valid, endpoint_identifier_is_valid, function_nodes_can_connect,
    page_identifier_is_valid, permission_identifier_is_valid, validate_route_path,
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

type LoweredProgram = (
    BTreeMap<SymbolId, CompiledPage>,
    BTreeMap<SymbolId, BytecodeSegment>,
    BTreeMap<SymbolId, BytecodeSegment>,
    Vec<CompiledRoute>,
);

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
}

include!("pipeline.rs");

include!("model_validation.rs");

include!("page_lowering.rs");

include!("reference_validation.rs");

include!("function_lowering.rs");

pub fn content_hash(definition: &ProgramDefinition) -> Result<String> {
    let bytes = serde_json::to_vec(definition)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    include!("tests.rs");

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
            "aio_first_party_api_keys"
        );
        assert_eq!(
            convention_page_path("aio-first-party", "API Keys"),
            "src/pages/aio_first_party_api_keys.rs"
        );
    }
}
