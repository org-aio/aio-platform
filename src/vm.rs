use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::{BytecodeInstruction, BytecodeSegment, Instruction, SymbolId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VmEffect {
    ValidateForm {
        rules: u32,
        value: Value,
    },
    CreateRecord {
        model_id: SymbolId,
        value: Value,
    },
    ReadRecord {
        model_id: SymbolId,
        value: Value,
    },
    UpdateRecord {
        model_id: SymbolId,
        value: Value,
    },
    DeleteRecord {
        model_id: SymbolId,
        value: Value,
    },
    QueryRecords {
        model_id: SymbolId,
        limit: u32,
        value: Value,
    },
    Navigate {
        route_id: SymbolId,
    },
    Confirm {
        value: Value,
    },
    Notify {
        level: String,
        value: Value,
    },
    Capability {
        capability_id: String,
        operation: String,
        value: Value,
    },
    InvokeServerSegment {
        segment_id: SymbolId,
        inputs: BTreeMap<SymbolId, Value>,
    },
}

/// 客户端和服务端分别实现 Effect，VM 本身不持有数据库、Secret 或浏览器对象。
#[allow(async_fn_in_trait)]
pub trait GraphVmHost {
    async fn apply(&mut self, effect: VmEffect) -> Result<Value>;
}

pub struct GraphVm<'a> {
    functions: &'a BTreeMap<SymbolId, BytecodeSegment>,
    max_steps: usize,
}

impl<'a> GraphVm<'a> {
    #[must_use]
    pub fn new(functions: &'a BTreeMap<SymbolId, BytecodeSegment>) -> Self {
        Self {
            functions,
            max_steps: 100_000,
        }
    }

    pub async fn execute<H: GraphVmHost>(
        &self,
        function_id: SymbolId,
        inputs: &BTreeMap<SymbolId, Value>,
        host: &mut H,
    ) -> Result<Value> {
        let segment = self
            .functions
            .get(&function_id)
            .with_context(|| format!("bytecode segment not found: {function_id}"))?;
        self.execute_segment(segment, inputs, host).await
    }

    async fn execute_segment<H: GraphVmHost>(
        &self,
        segment: &BytecodeSegment,
        inputs: &BTreeMap<SymbolId, Value>,
        host: &mut H,
    ) -> Result<Value> {
        if segment.instructions.len() > self.max_steps {
            bail!("bytecode 超过单次执行步数上限");
        }
        let mut registers = vec![Value::Null; segment.instructions.len()];
        let mut result = Value::Null;
        for encoded in &segment.instructions {
            let value = self
                .execute_instruction(encoded, &registers, &segment.constants, inputs, host)
                .await
                .with_context(|| format!("执行 bytecode 节点失败: {}", encoded.node_id))?;
            if let Some(slot) = encoded.output_slot
                && let Some(register) = registers.get_mut(slot as usize)
            {
                *register = value.clone();
            }
            result = value;
            if matches!(encoded.instruction, Instruction::Return) {
                break;
            }
        }
        Ok(result)
    }

    async fn execute_instruction<H: GraphVmHost>(
        &self,
        encoded: &BytecodeInstruction,
        registers: &[Value],
        constants: &[Value],
        inputs: &BTreeMap<SymbolId, Value>,
        host: &mut H,
    ) -> Result<Value> {
        let values = encoded
            .input_slots
            .iter()
            .filter_map(|(name, slot)| {
                registers
                    .get(*slot as usize)
                    .cloned()
                    .map(|value| (name.as_str(), value))
            })
            .collect::<BTreeMap<_, _>>();
        let first = || values.values().next().cloned().unwrap_or(Value::Null);
        match &encoded.instruction {
            Instruction::LoadConstant { constant, .. } => constants
                .get(*constant as usize)
                .cloned()
                .with_context(|| format!("constant slot out of bounds: {constant}")),
            Instruction::LoadInput { port_id, .. } => inputs
                .get(port_id)
                .cloned()
                .with_context(|| format!("function input missing: {port_id}")),
            Instruction::MakeObject { fields, .. } => {
                let mut object = Map::new();
                for (field, value) in fields.iter().zip(values.values()) {
                    object.insert(field.to_string(), value.clone());
                }
                Ok(Value::Object(object))
            }
            Instruction::MakeList { .. } => Ok(Value::Array(values.into_values().collect())),
            Instruction::ReadField { field_id, .. } => first()
                .get(field_id.to_string())
                .cloned()
                .with_context(|| format!("对象字段不存在: {field_id}")),
            Instruction::Format { template, .. } => {
                let mut output = template.clone();
                for (index, value) in values.values().enumerate() {
                    output = output.replace(&format!("{{{index}}}"), &display_value(value));
                }
                Ok(Value::String(output))
            }
            Instruction::Compare { operator, .. } => compare(operator, values.values()),
            Instruction::Boolean { operator, .. } => boolean(operator, values.values()),
            Instruction::Math { operator, .. } => math(operator, values.values()),
            Instruction::Branch { .. } => {
                let condition = values
                    .get("condition")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Ok(values
                    .get(if condition { "then" } else { "else" })
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            Instruction::ForEach {
                max_items,
                body_function_id,
            } => {
                let items = first().as_array().cloned().unwrap_or_default();
                if items.len() > *max_items as usize {
                    bail!("遍历数量超过编译上限: {max_items}");
                }
                let body = self
                    .functions
                    .get(body_function_id)
                    .with_context(|| format!("遍历函数不存在: {body_function_id}"))?;
                let input_port = body.instructions.iter().find_map(|instruction| {
                    if let Instruction::LoadInput { port_id, .. } = instruction.instruction {
                        Some(port_id)
                    } else {
                        None
                    }
                });
                let mut output = Vec::with_capacity(items.len());
                for item in items {
                    let body_inputs = input_port
                        .map(|port| BTreeMap::from([(port, item)]))
                        .unwrap_or_default();
                    output.push(Box::pin(self.execute_segment(body, &body_inputs, host)).await?);
                }
                Ok(Value::Array(output))
            }
            Instruction::ValidateForm { rule_count } => {
                host.apply(VmEffect::ValidateForm {
                    rules: *rule_count,
                    value: first(),
                })
                .await
            }
            Instruction::CreateRecord { model_id } => {
                host.apply(VmEffect::CreateRecord {
                    model_id: *model_id,
                    value: first(),
                })
                .await
            }
            Instruction::ReadRecord { model_id } => {
                host.apply(VmEffect::ReadRecord {
                    model_id: *model_id,
                    value: first(),
                })
                .await
            }
            Instruction::UpdateRecord { model_id } => {
                host.apply(VmEffect::UpdateRecord {
                    model_id: *model_id,
                    value: first(),
                })
                .await
            }
            Instruction::DeleteRecord { model_id } => {
                host.apply(VmEffect::DeleteRecord {
                    model_id: *model_id,
                    value: first(),
                })
                .await
            }
            Instruction::QueryRecords { model_id, limit } => {
                host.apply(VmEffect::QueryRecords {
                    model_id: *model_id,
                    limit: *limit,
                    value: first(),
                })
                .await
            }
            Instruction::Navigate { route_id } => {
                host.apply(VmEffect::Navigate {
                    route_id: *route_id,
                })
                .await
            }
            Instruction::Confirm => host.apply(VmEffect::Confirm { value: first() }).await,
            Instruction::Notify { level } => {
                host.apply(VmEffect::Notify {
                    level: level.clone(),
                    value: first(),
                })
                .await
            }
            Instruction::InvokeCapability {
                capability_id,
                operation,
            } => {
                host.apply(VmEffect::Capability {
                    capability_id: capability_id.clone(),
                    operation: operation.clone(),
                    value: first(),
                })
                .await
            }
            Instruction::InvokeServerSegment {
                segment_id,
                input_port,
            } => {
                host.apply(VmEffect::InvokeServerSegment {
                    segment_id: *segment_id,
                    inputs: BTreeMap::from([(*input_port, first())]),
                })
                .await
            }
            Instruction::Return => Ok(first()),
            Instruction::Fail { code } => bail!("graph failed: {code}"),
        }
    }
}

fn compare<'a>(operator: &str, mut values: impl Iterator<Item = &'a Value>) -> Result<Value> {
    let left = values.next().cloned().unwrap_or(Value::Null);
    let right = values.next().cloned().unwrap_or(Value::Null);
    let result = match operator {
        "equal" => left == right,
        "notequal" => left != right,
        "greater" => number(&left)? > number(&right)?,
        "greaterorequal" => number(&left)? >= number(&right)?,
        "less" => number(&left)? < number(&right)?,
        "lessorequal" => number(&left)? <= number(&right)?,
        "contains" => display_value(&left).contains(&display_value(&right)),
        _ => bail!("未知比较运算: {operator}"),
    };
    Ok(Value::Bool(result))
}

fn boolean<'a>(operator: &str, values: impl Iterator<Item = &'a Value>) -> Result<Value> {
    let values = values
        .map(|value| value.as_bool().unwrap_or(false))
        .collect::<Vec<_>>();
    let result = match operator {
        "and" => values.iter().all(|value| *value),
        "or" => values.iter().any(|value| *value),
        "not" => !values.first().copied().unwrap_or(false),
        _ => bail!("未知布尔运算: {operator}"),
    };
    Ok(Value::Bool(result))
}

fn math<'a>(operator: &str, mut values: impl Iterator<Item = &'a Value>) -> Result<Value> {
    let null = Value::Null;
    let left = number(values.next().unwrap_or(&null))?;
    let right = number(values.next().unwrap_or(&null))?;
    let result = match operator {
        "add" => left + right,
        "subtract" => left - right,
        "multiply" => left * right,
        "divide" if right != 0.0 => left / right,
        "remainder" if right != 0.0 => left % right,
        "divide" | "remainder" => bail!("数学运算不能除以零"),
        _ => bail!("未知数学运算: {operator}"),
    };
    Number::from_f64(result)
        .map(Value::Number)
        .context("数学运算产生了非有限值")
}

fn number(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .with_context(|| format!("值不是数字: {value}"))
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BytecodeInstruction, BytecodeSegment, EffectKind};
    use serde_json::json;

    struct TestHost;

    impl GraphVmHost for TestHost {
        async fn apply(&mut self, _effect: VmEffect) -> Result<Value> {
            Ok(json!(true))
        }
    }

    #[tokio::test]
    async fn executes_constant_math_and_return() -> Result<()> {
        let function_id = SymbolId::new();
        let instructions = vec![
            BytecodeInstruction {
                node_id: SymbolId::new(),
                input_slots: BTreeMap::new(),
                output_slot: Some(0),
                instruction: Instruction::LoadConstant {
                    slot: 0,
                    constant: 0,
                },
            },
            BytecodeInstruction {
                node_id: SymbolId::new(),
                input_slots: BTreeMap::new(),
                output_slot: Some(1),
                instruction: Instruction::LoadConstant {
                    slot: 1,
                    constant: 1,
                },
            },
            BytecodeInstruction {
                node_id: SymbolId::new(),
                input_slots: BTreeMap::from([("left".to_owned(), 0), ("right".to_owned(), 1)]),
                output_slot: Some(2),
                instruction: Instruction::Math {
                    slot: 2,
                    operator: "add".to_owned(),
                },
            },
            BytecodeInstruction {
                node_id: SymbolId::new(),
                input_slots: BTreeMap::from([("value".to_owned(), 2)]),
                output_slot: None,
                instruction: Instruction::Return,
            },
        ];
        let functions = BTreeMap::from([(
            function_id,
            BytecodeSegment {
                id: function_id,
                name: "sum".to_owned(),
                input_ports: BTreeMap::new(),
                effects: Vec::<EffectKind>::new(),
                instructions,
                constants: vec![json!(20), json!(22)],
            },
        )]);
        let mut host = TestHost;
        let result = GraphVm::new(&functions)
            .execute(function_id, &BTreeMap::new(), &mut host)
            .await?;
        assert_eq!(result, json!(42.0));
        Ok(())
    }
}
