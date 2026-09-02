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

