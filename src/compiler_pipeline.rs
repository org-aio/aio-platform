impl<'a> ProgramCompiler<'a> {
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
    ) -> LoweredProgram {
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
