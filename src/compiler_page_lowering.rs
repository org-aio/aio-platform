#[must_use]
pub fn compile_page(definition: &ProgramDefinition, page: &PageDefinition) -> CompiledPage {
    let renderer = match &page.renderer {
        PageRendererDefinition::ConventionFile => CompiledPageRenderer::ConventionFile {
            module_name: convention_page_module_name(&definition.name, &page.name),
            expected_path: convention_page_path(&definition.name, &page.name),
        },
        PageRendererDefinition::MenuTree => CompiledPageRenderer::MenuTree,
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
                source: PageEndpointSource::Convention,
            }),
    );
    endpoints
}

fn built_in_page_endpoints(
    definition: &ProgramDefinition,
    page: &PageDefinition,
) -> Vec<CompiledPageEndpoint> {
    let table = match &page.renderer {
        PageRendererDefinition::CrudTable { table } => table,
        PageRendererDefinition::TreeTable { table, .. } => table,
        PageRendererDefinition::ConventionFile | PageRendererDefinition::MenuTree => {
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
    format!("{program_name}_{page_name}")
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
