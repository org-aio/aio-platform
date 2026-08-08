use std::{env, fmt};

use anyhow::{Context, Result, bail};
use rig::{
    client::CompletionClient,
    completion::{Completion, ToolDefinition},
    message::{AssistantContent, ToolCall, ToolChoice, ToolFunction},
    providers::openai,
    tool::Tool,
};
use serde_json::{Map, Value, json};

use crate::{CompiledModel, FormStateExtractionResponse, ValueType};

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-5.5";

#[derive(Clone, Debug)]
pub struct FormStateExtractor {
    config: Option<FormStateAgentConfig>,
}

#[derive(Clone, Debug)]
struct FormStateAgentConfig {
    api_key: String,
    api_base: String,
    model: String,
}

#[derive(Clone)]
struct SubmitFormState {
    schema: Value,
    model: CompiledModel,
}

#[derive(Debug)]
struct FormStateValidationError(String);

impl fmt::Display for FormStateValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "formState 不符合页面字段定义: {}", self.0)
    }
}

impl std::error::Error for FormStateValidationError {}

impl FormStateExtractor {
    pub fn from_env() -> Result<Self> {
        let Some(api_key) = first_env(["OPENAI_API_KEY", "API_KEY"]) else {
            return Ok(Self { config: None });
        };
        let api_base = first_env(["OPENAI_BASE_URL", "OPENAI_BASEURL", "API_BASEURL"])
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let api_base = normalize_api_base(&api_base)?;
        let model = first_env(["AZ_AIO_FORM_STATE_MODEL", "OPENAI_MODEL"])
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        Ok(Self {
            config: Some(FormStateAgentConfig {
                api_key,
                api_base,
                model,
            }),
        })
    }

    pub async fn extract(
        &self,
        compiled_model: &CompiledModel,
        prompt: &str,
        current_form_state: &Value,
        model_override: Option<&str>,
    ) -> Result<FormStateExtractionResponse> {
        let config = self
            .config
            .as_ref()
            .context("AI 表单填写需要 OPENAI_API_KEY")?;
        let model = model_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&config.model);
        let schema = form_state_schema(compiled_model);
        let submit = SubmitFormState {
            schema,
            model: compiled_model.clone(),
        };
        let client = openai::Client::builder()
            .api_key(&config.api_key)
            .base_url(&versioned_api_base(&config.api_base))
            .build()
            .context("创建 formState OpenAI client 失败")?
            .completions_api();
        let input = serde_json::to_string(&json!({
            "request": prompt,
            "current_form_state": current_form_state,
            "model_name": compiled_model.name,
            "model_title": compiled_model.title,
        }))
        .context("序列化 formState 提取输入失败")?;
        let agent = client
            .agent(model)
            .preamble(
                "你负责把用户输入转换为页面 formState。只调用 submit_form_state，\
                 只提交用户明确给出或能够可靠推断的字段；不要编造缺失值。",
            )
            .tool(submit)
            .tool_choice(ToolChoice::Required)
            .additional_params(json!({
                "parallel_tool_calls": false
            }))
            .max_tokens(4_096)
            .build();
        let history = Vec::<rig::message::Message>::new();
        let response = agent
            .completion(input, &history)
            .await
            .with_context(|| format!("创建 AI formState completion 失败: {model}"))?
            .send()
            .await
            .with_context(|| format!("AI formState 提取失败: {model}"))?;
        let mut form_state = response
            .choice
            .into_iter()
            .find_map(|content| match content {
                AssistantContent::ToolCall(ToolCall {
                    function:
                        ToolFunction {
                            name, arguments, ..
                        },
                    ..
                }) if name == SubmitFormState::NAME => Some(arguments),
                _ => None,
            })
            .context("AI 未调用 submit_form_state")?;
        validate_form_state(compiled_model, &form_state)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if let Some(values) = form_state.as_object_mut() {
            values.retain(|_, value| !value.is_null());
        }
        Ok(FormStateExtractionResponse {
            form_state,
            model: model.to_owned(),
        })
    }
}

impl Tool for SubmitFormState {
    const NAME: &'static str = "submit_form_state";

    type Error = FormStateValidationError;
    type Args = Value;
    type Output = Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_owned(),
            description: "提交符合当前页面字段定义的部分 formState".to_owned(),
            parameters: self.schema.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        validate_form_state(&self.model, &args)?;
        Ok(args)
    }
}

pub(crate) fn form_state_schema(model: &CompiledModel) -> Value {
    let mut properties = Map::new();
    for (slot, name) in &model.field_names {
        let Some(options) = model.field_options.get(slot) else {
            continue;
        };
        if !options.form_visible || !options.ai_extract || !options.form_editable {
            continue;
        }
        let mut value_schema =
            json_schema_for_type(model.field_types.get(slot).unwrap_or(&ValueType::Any));
        let Some(schema) = value_schema.as_object_mut() else {
            continue;
        };
        let validation = &options.validation;
        for (key, value) in [
            ("minLength", validation.min_length.map(Value::from)),
            ("maxLength", validation.max_length.map(Value::from)),
            ("minimum", validation.minimum.map(Value::from)),
            ("maximum", validation.maximum.map(Value::from)),
            ("pattern", validation.pattern.clone().map(Value::String)),
        ] {
            if let Some(value) = value {
                schema.insert(key.to_owned(), value);
            }
        }
        let mut field_schema = json!({
            "anyOf": [value_schema, {"type": "null"}],
        });
        let schema = field_schema
            .as_object_mut()
            .expect("字段 Schema 必须是对象");
        if let Some(title) = model.field_titles.get(slot) {
            schema.insert("title".to_owned(), Value::String(title.clone()));
        }
        if let Some(help_text) = &options.help_text {
            schema.insert("description".to_owned(), Value::String(help_text.clone()));
        }
        if let Some(default_value) = &options.default_value {
            schema.insert("default".to_owned(), default_value.clone());
        }
        properties.insert(name.clone(), field_schema);
    }
    json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false
    })
}

fn json_schema_for_type(value_type: &ValueType) -> Value {
    match value_type {
        ValueType::Null => json!({"type": "null"}),
        ValueType::Boolean => json!({"type": "boolean"}),
        ValueType::Integer | ValueType::TimestampMs => json!({"type": "integer"}),
        ValueType::Decimal => json!({"type": "number"}),
        ValueType::Text | ValueType::File => json!({"type": "string"}),
        ValueType::Object { .. } => json!({
            "type": "string",
            "description": "关联记录的稳定 ID",
        }),
        ValueType::List { item } => json!({
            "type": "array",
            "items": json_schema_for_type(item),
        }),
        ValueType::Optional { value } => json!({
            "anyOf": [json_schema_for_type(value), {"type": "null"}],
        }),
        ValueType::Any => json!({}),
    }
}

fn validate_form_state(
    model: &CompiledModel,
    form_state: &Value,
) -> std::result::Result<(), FormStateValidationError> {
    let object = form_state
        .as_object()
        .ok_or_else(|| FormStateValidationError("必须是 JSON 对象".to_owned()))?;
    for (name, value) in object {
        let Some(slot) = model
            .field_names
            .iter()
            .find_map(|(slot, field_name)| (field_name == name).then_some(*slot))
        else {
            return Err(FormStateValidationError(format!("未知字段 {name}")));
        };
        let options = model
            .field_options
            .get(&slot)
            .ok_or_else(|| FormStateValidationError(format!("字段 {name} 缺少配置")))?;
        if !options.form_visible || !options.ai_extract || !options.form_editable {
            return Err(FormStateValidationError(format!(
                "字段 {name} 不允许 AI 填写"
            )));
        }
        if value.is_null() {
            continue;
        }
        let value_type = model
            .field_types
            .get(&slot)
            .ok_or_else(|| FormStateValidationError(format!("字段 {name} 缺少类型")))?;
        if !value_matches_type(value_type, value) {
            return Err(FormStateValidationError(format!("字段 {name} 类型不匹配")));
        }
        validate_field_constraints(name, &options.validation, value)?;
    }
    Ok(())
}

fn validate_field_constraints(
    name: &str,
    validation: &crate::FieldValidation,
    value: &Value,
) -> std::result::Result<(), FormStateValidationError> {
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u32;
        if validation
            .min_length
            .is_some_and(|minimum| length < minimum)
        {
            return Err(FormStateValidationError(format!(
                "字段 {name} 小于最小长度"
            )));
        }
        if validation
            .max_length
            .is_some_and(|maximum| length > maximum)
        {
            return Err(FormStateValidationError(format!(
                "字段 {name} 超过最大长度"
            )));
        }
        if let Some(pattern) = validation.pattern.as_deref() {
            let pattern = regex::Regex::new(pattern).map_err(|error| {
                FormStateValidationError(format!("字段 {name} 的正则表达式无效: {error}"))
            })?;
            if !pattern.is_match(text) {
                return Err(FormStateValidationError(format!(
                    "字段 {name} 不符合格式要求"
                )));
            }
        }
    }
    if let Some(number) = value.as_f64() {
        if validation.minimum.is_some_and(|minimum| number < minimum) {
            return Err(FormStateValidationError(format!("字段 {name} 小于最小值")));
        }
        if validation.maximum.is_some_and(|maximum| number > maximum) {
            return Err(FormStateValidationError(format!("字段 {name} 超过最大值")));
        }
    }
    Ok(())
}

fn value_matches_type(value_type: &ValueType, value: &Value) -> bool {
    if value.is_null() {
        return matches!(
            value_type,
            ValueType::Null | ValueType::Optional { .. } | ValueType::Any
        );
    }
    match value_type {
        ValueType::Any => true,
        ValueType::Null => false,
        ValueType::Boolean => value.is_boolean(),
        ValueType::Integer | ValueType::TimestampMs => value.is_i64() || value.is_u64(),
        ValueType::Decimal => value.is_number(),
        ValueType::Text | ValueType::File => value.is_string(),
        ValueType::Object { .. } => value.is_object() || value.is_string(),
        ValueType::List { .. } => value.is_array(),
        ValueType::Optional { value: inner } => value_matches_type(inner, value),
    }
}

fn first_env<const N: usize>(names: [&str; N]) -> Option<String> {
    names.into_iter().find_map(|name| {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn normalize_api_base(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');
    if !value.starts_with("https://") && !value.starts_with("http://") {
        bail!("OPENAI_BASE_URL 必须是 HTTP(S) URL");
    }
    Ok(value.to_owned())
}

fn versioned_api_base(value: &str) -> String {
    if value.ends_with("/v1") {
        value.to_owned()
    } else {
        format!("{value}/v1")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{FieldOptions, FieldValidation, SymbolId};

    use super::*;

    #[test]
    fn schema_uses_only_ai_editable_form_fields() {
        let included_id = SymbolId::new();
        let excluded_id = SymbolId::new();
        let mut field_slots = BTreeMap::new();
        field_slots.insert(included_id, 0);
        field_slots.insert(excluded_id, 1);
        let model = CompiledModel {
            id: SymbolId::new(),
            name: "user".to_owned(),
            title: "用户".to_owned(),
            field_slots,
            field_types: BTreeMap::from([(0, ValueType::Text), (1, ValueType::Text)]),
            field_names: BTreeMap::from([(0, "username".to_owned()), (1, "secret".to_owned())]),
            field_titles: BTreeMap::from([(0, "用户名".to_owned()), (1, "密钥".to_owned())]),
            field_options: BTreeMap::from([
                (
                    0,
                    FieldOptions {
                        validation: FieldValidation {
                            min_length: Some(2),
                            ..FieldValidation::default()
                        },
                        ..FieldOptions::default()
                    },
                ),
                (
                    1,
                    FieldOptions {
                        ai_extract: false,
                        ..FieldOptions::default()
                    },
                ),
            ]),
            field_relations: BTreeMap::new(),
            required_fields: vec![0],
            expression_indexes: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
        };
        let schema = form_state_schema(&model);
        assert_eq!(
            schema.pointer("/properties/username/anyOf/0/minLength"),
            Some(&json!(2))
        );
        assert!(schema.pointer("/properties/secret").is_none());
        assert!(validate_form_state(&model, &json!({"username": "x"})).is_err());
        assert!(validate_form_state(&model, &json!({"username": "xy"})).is_ok());
    }

    #[tokio::test]
    #[ignore = "需要真实 AI provider"]
    async fn live_provider_extracts_dynamic_form_state() -> anyhow::Result<()> {
        let username_id = SymbolId::new();
        let display_name_id = SymbolId::new();
        let model = CompiledModel {
            id: SymbolId::new(),
            name: "user".to_owned(),
            title: "用户".to_owned(),
            field_slots: BTreeMap::from([(username_id, 0), (display_name_id, 1)]),
            field_types: BTreeMap::from([(0, ValueType::Text), (1, ValueType::Text)]),
            field_names: BTreeMap::from([
                (0, "username".to_owned()),
                (1, "display_name".to_owned()),
            ]),
            field_titles: BTreeMap::from([(0, "用户名".to_owned()), (1, "姓名".to_owned())]),
            field_options: BTreeMap::from([
                (0, FieldOptions::default()),
                (1, FieldOptions::default()),
            ]),
            field_relations: BTreeMap::new(),
            required_fields: vec![0, 1],
            expression_indexes: Vec::new(),
            audit: crate::ModelAuditDefinition::default(),
        };
        let response = FormStateExtractor::from_env()?
            .extract(&model, "用户名 test_ai，姓名张三", &json!({}), None)
            .await?;
        assert_eq!(response.form_state["username"], "test_ai");
        assert_eq!(response.form_state["display_name"], "张三");
        Ok(())
    }
}
