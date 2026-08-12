use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::{CompiledModel, FieldRelation, RelationKind, RuntimeRecordView, ValueType};

pub(crate) fn record_payload_from_state(
    model: &CompiledModel,
    form_state: &BTreeMap<String, String>,
) -> Result<Value, String> {
    let mut payload = Map::new();
    for (slot, name) in &model.field_names {
        if !model
            .field_options
            .get(slot)
            .is_some_and(|options| options.form_visible)
        {
            continue;
        }
        let title = model
            .field_titles
            .get(slot)
            .map_or(name.as_str(), String::as_str);
        let value_type = model
            .field_types
            .get(slot)
            .ok_or_else(|| format!("字段“{title}”缺少编译类型"))?;
        let raw = form_state.get(name).map_or("", String::as_str);
        let required = model.required_fields.contains(slot);
        let value = match model.field_relations.get(slot) {
            Some(relation) => parse_relation_field_value(relation, raw, title, required)?,
            None => parse_field_value(value_type, raw, title, required)?,
        };
        payload.insert(name.clone(), value);
    }
    Ok(Value::Object(payload))
}

pub(crate) fn selected_relation_ids(
    relation: &FieldRelation,
    raw: &str,
    title: &str,
) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if !relation.kind.is_collection() {
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return Err(format!("字段“{title}”必须选择稳定记录 ID"));
        }
        return Ok(vec![trimmed.to_owned()]);
    }
    let ids = serde_json::from_str::<Vec<String>>(trimmed)
        .map_err(|_| format!("字段“{title}”的关系值必须是 ID 数组"))?;
    let mut unique = BTreeSet::new();
    for id in &ids {
        if id.trim().is_empty() {
            return Err(format!("字段“{title}”不能包含空记录 ID"));
        }
        if !unique.insert(id.as_str()) {
            return Err(format!("字段“{title}”不能包含重复记录"));
        }
    }
    Ok(ids)
}

pub(crate) fn relation_form_state_value(kind: RelationKind, ids: Vec<String>) -> String {
    if kind.is_collection() {
        return Value::Array(ids.into_iter().map(Value::String).collect()).to_string();
    }
    ids.into_iter().next().map_or_else(String::new, |id| id)
}

pub(crate) fn relation_record_label(model: &CompiledModel, record: &RuntimeRecordView) -> String {
    let mut labels = Vec::new();
    for slot in readable_relation_slots(model) {
        let Some(name) = model.field_names.get(&slot) else {
            continue;
        };
        let Some(value) = record.payload.get(name).and_then(relation_label_value) else {
            continue;
        };
        if !labels.contains(&value) {
            labels.push(value);
        }
        if labels.len() == 2 {
            break;
        }
    }
    if labels.is_empty() {
        record.id.clone()
    } else {
        labels.join(" · ")
    }
}

pub(crate) fn relation_search_fields(model: &CompiledModel) -> Vec<String> {
    readable_relation_slots(model)
        .into_iter()
        .filter(|slot| {
            model
                .field_types
                .get(slot)
                .is_some_and(searchable_relation_type)
        })
        .filter_map(|slot| model.field_names.get(&slot).cloned())
        .collect()
}

fn readable_relation_slots(model: &CompiledModel) -> Vec<u32> {
    const PREFERRED_NAMES: [&str; 5] = ["name", "title", "display_name", "username", "code"];
    let mut slots = Vec::new();
    for preferred in PREFERRED_NAMES {
        if let Some(slot) = model
            .field_names
            .iter()
            .find_map(|(slot, name)| (name == preferred).then_some(*slot))
        {
            slots.push(slot);
        }
    }
    slots.extend(model.field_names.keys().copied());
    let mut visited = BTreeSet::new();
    slots
        .into_iter()
        .filter(|slot| {
            visited.insert(*slot)
                && model
                    .field_options
                    .get(slot)
                    .is_some_and(|options| options.list_visible)
        })
        .collect()
}

fn searchable_relation_type(value_type: &ValueType) -> bool {
    match value_type {
        ValueType::Boolean
        | ValueType::Integer
        | ValueType::Decimal
        | ValueType::Text
        | ValueType::TimestampMs
        | ValueType::File => true,
        ValueType::Optional { value } => searchable_relation_type(value),
        ValueType::Any | ValueType::Null | ValueType::Object { .. } | ValueType::List { .. } => {
            false
        }
    }
}

fn parse_relation_field_value(
    relation: &FieldRelation,
    raw: &str,
    title: &str,
    required: bool,
) -> Result<Value, String> {
    let ids = selected_relation_ids(relation, raw, title)?;
    if required && ids.is_empty() {
        return Err(format!("字段“{title}”不能为空"));
    }
    if relation.kind.is_collection() {
        return Ok(Value::Array(ids.into_iter().map(Value::String).collect()));
    }
    Ok(ids.into_iter().next().map_or(Value::Null, Value::String))
}

fn relation_label_value(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

fn parse_field_value(
    value_type: &ValueType,
    raw: &str,
    title: &str,
    required: bool,
) -> Result<Value, String> {
    let trimmed = raw.trim();
    if let ValueType::Optional { value } = value_type {
        return if trimmed.is_empty() {
            Ok(Value::Null)
        } else {
            parse_field_value(value, raw, title, false)
        };
    }
    if required && trimmed.is_empty() && !matches!(value_type, ValueType::Boolean) {
        return Err(format!("字段“{title}”不能为空"));
    }
    match value_type {
        ValueType::Any => {
            if trimmed.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned())))
            }
        }
        ValueType::Null => Ok(Value::Null),
        ValueType::Boolean => match trimmed {
            "true" | "on" | "1" => Ok(Value::Bool(true)),
            "false" | "off" | "0" | "" => Ok(Value::Bool(false)),
            _ => Err(format!("字段“{title}”必须是布尔值")),
        },
        ValueType::Integer | ValueType::TimestampMs => {
            if trimmed.is_empty() {
                return Ok(Value::Null);
            }
            trimmed
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| format!("字段“{title}”必须是整数"))
        }
        ValueType::Decimal => {
            if trimmed.is_empty() {
                return Ok(Value::Null);
            }
            let number = trimmed
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .ok_or_else(|| format!("字段“{title}”必须是有限小数"))?;
            Ok(Value::Number(number))
        }
        ValueType::Text | ValueType::File => Ok(Value::String(raw.to_owned())),
        ValueType::Object { .. } => {
            let value = parse_json_value(raw, title, "JSON 对象", required)?;
            if value.is_object() || value.is_null() && !required {
                Ok(value)
            } else {
                Err(format!("字段“{title}”必须是 JSON 对象"))
            }
        }
        ValueType::List { .. } => {
            let value = parse_json_value(raw, title, "JSON 数组", required)?;
            if value.is_array() || value.is_null() && !required {
                Ok(value)
            } else {
                Err(format!("字段“{title}”必须是 JSON 数组"))
            }
        }
        ValueType::Optional { .. } => unreachable!(),
    }
}

fn parse_json_value(
    raw: &str,
    title: &str,
    expected: &str,
    required: bool,
) -> Result<Value, String> {
    if raw.trim().is_empty() && !required {
        return Ok(Value::Null);
    }
    serde_json::from_str(raw).map_err(|_| format!("字段“{title}”必须是 {expected}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldOptions, ModelAuditDefinition, SymbolId};
    use serde_json::json;

    fn model(types: Vec<ValueType>, required_fields: Vec<u32>) -> CompiledModel {
        let field_names = types
            .iter()
            .enumerate()
            .map(|(slot, _)| (slot as u32, format!("field_{slot}")))
            .collect::<BTreeMap<_, _>>();
        let field_titles = types
            .iter()
            .enumerate()
            .map(|(slot, _)| (slot as u32, format!("字段 {slot}")))
            .collect::<BTreeMap<_, _>>();
        let field_options = types
            .iter()
            .enumerate()
            .map(|(slot, _)| (slot as u32, FieldOptions::default()))
            .collect::<BTreeMap<_, _>>();
        CompiledModel {
            id: SymbolId::new(),
            name: "record".to_owned(),
            title: "记录".to_owned(),
            primary_key: crate::ModelPrimaryKeyDefinition::default(),
            field_slots: BTreeMap::new(),
            field_types: types
                .into_iter()
                .enumerate()
                .map(|(slot, value_type)| (slot as u32, value_type))
                .collect(),
            field_names,
            field_titles,
            field_options,
            field_relations: BTreeMap::new(),
            required_fields,
            expression_indexes: Vec::new(),
            audit: ModelAuditDefinition::default(),
        }
    }

    #[test]
    fn rejects_invalid_integer_and_json_values() {
        let model = model(
            vec![
                ValueType::Integer,
                ValueType::Object {
                    model_id: SymbolId::new(),
                },
                ValueType::List {
                    item: Box::new(ValueType::Text),
                },
            ],
            vec![],
        );

        let integer_error = record_payload_from_state(
            &model,
            &BTreeMap::from([
                ("field_0".to_owned(), "not-a-number".to_owned()),
                ("field_1".to_owned(), "{}".to_owned()),
                ("field_2".to_owned(), "[]".to_owned()),
            ]),
        )
        .unwrap_err();
        assert_eq!(integer_error, "字段“字段 0”必须是整数");

        let object_error = record_payload_from_state(
            &model,
            &BTreeMap::from([
                ("field_0".to_owned(), "1".to_owned()),
                ("field_1".to_owned(), "[]".to_owned()),
                ("field_2".to_owned(), "[]".to_owned()),
            ]),
        )
        .unwrap_err();
        assert_eq!(object_error, "字段“字段 1”必须是 JSON 对象");

        let list_error = record_payload_from_state(
            &model,
            &BTreeMap::from([
                ("field_0".to_owned(), "1".to_owned()),
                ("field_1".to_owned(), "{}".to_owned()),
                ("field_2".to_owned(), "not-json".to_owned()),
            ]),
        )
        .unwrap_err();
        assert_eq!(list_error, "字段“字段 2”必须是 JSON 数组");
    }

    #[test]
    fn converts_valid_typed_values_and_optional_empty_value() {
        let model = model(
            vec![
                ValueType::Boolean,
                ValueType::Integer,
                ValueType::Decimal,
                ValueType::Object {
                    model_id: SymbolId::new(),
                },
                ValueType::List {
                    item: Box::new(ValueType::Integer),
                },
                ValueType::Optional {
                    value: Box::new(ValueType::TimestampMs),
                },
                ValueType::Any,
            ],
            vec![1],
        );
        let payload = record_payload_from_state(
            &model,
            &BTreeMap::from([
                ("field_0".to_owned(), "true".to_owned()),
                ("field_1".to_owned(), "42".to_owned()),
                ("field_2".to_owned(), "1.25".to_owned()),
                ("field_3".to_owned(), r#"{"id":"dept"}"#.to_owned()),
                ("field_4".to_owned(), "[1,2]".to_owned()),
                ("field_5".to_owned(), String::new()),
                ("field_6".to_owned(), "plain text".to_owned()),
            ]),
        )
        .unwrap();

        assert_eq!(
            payload,
            json!({
                "field_0": true,
                "field_1": 42,
                "field_2": 1.25,
                "field_3": {"id": "dept"},
                "field_4": [1, 2],
                "field_5": null,
                "field_6": "plain text",
            })
        );
    }

    #[test]
    fn rejects_empty_required_value() {
        let text_model = model(vec![ValueType::Text], vec![0]);
        let text_error = record_payload_from_state(&text_model, &BTreeMap::new()).unwrap_err();
        assert_eq!(text_error, "字段“字段 0”不能为空");

        let object_model = model(
            vec![ValueType::Object {
                model_id: SymbolId::new(),
            }],
            vec![0],
        );
        let object_error = record_payload_from_state(
            &object_model,
            &BTreeMap::from([("field_0".to_owned(), "null".to_owned())]),
        )
        .unwrap_err();
        assert_eq!(object_error, "字段“字段 0”必须是 JSON 对象");
    }

    #[test]
    fn relation_fields_store_stable_record_ids() -> Result<(), String> {
        let target_model_id = SymbolId::new();
        let mut model = model(
            vec![
                ValueType::Object {
                    model_id: target_model_id,
                },
                ValueType::List {
                    item: Box::new(ValueType::Object {
                        model_id: target_model_id,
                    }),
                },
            ],
            vec![0],
        );
        model.field_relations = BTreeMap::from([
            (
                0,
                FieldRelation {
                    kind: RelationKind::ManyToOne,
                    target_model_id,
                    target_field_id: SymbolId::new(),
                },
            ),
            (
                1,
                FieldRelation {
                    kind: RelationKind::ManyToMany,
                    target_model_id,
                    target_field_id: SymbolId::new(),
                },
            ),
        ]);

        let payload = record_payload_from_state(
            &model,
            &BTreeMap::from([
                ("field_0".to_owned(), "department-1".to_owned()),
                ("field_1".to_owned(), "[\"role-1\",\"role-2\"]".to_owned()),
            ]),
        )?;
        assert_eq!(
            payload,
            json!({
                "field_0": "department-1",
                "field_1": ["role-1", "role-2"],
            })
        );

        let error = record_payload_from_state(
            &model,
            &BTreeMap::from([
                ("field_0".to_owned(), r#"{"id":"department-1"}"#.to_owned()),
                ("field_1".to_owned(), "[]".to_owned()),
            ]),
        )
        .unwrap_err();
        assert_eq!(error, "字段“字段 0”必须选择稳定记录 ID");
        assert_eq!(
            relation_form_state_value(
                RelationKind::ManyToMany,
                vec!["role-1".to_owned(), "role-2".to_owned()],
            ),
            "[\"role-1\",\"role-2\"]"
        );
        Ok(())
    }

    #[test]
    fn relation_option_label_prefers_readable_list_fields() {
        let model = model(vec![ValueType::Text, ValueType::Text], vec![]);
        let record = RuntimeRecordView {
            id: "department-1".to_owned(),
            payload: json!({
                "field_0": "研发部",
                "field_1": "RND",
            }),
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        assert_eq!(relation_record_label(&model, &record), "研发部 · RND");
        assert_eq!(
            relation_search_fields(&model),
            vec!["field_0".to_owned(), "field_1".to_owned()]
        );
    }
}
