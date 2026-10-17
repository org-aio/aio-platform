use anyhow::{Result, ensure};
use serde_json::Value;

/// 所有执行入口共用的有界 JSON 契约；装配时编译，调用时复用，不访问文件或网络。
pub struct ValueSchema(Option<jsonschema::Validator>);

impl ValueSchema {
    /// 未声明 schema 表示任意 JSON；已声明契约固定采用 2020-12。
    pub fn compile(schema: Option<&schemars::Schema>) -> Result<Self> {
        let Some(schema) = schema else {
            return Ok(Self(None));
        };
        let value = schema.as_value();
        ensure!(
            serde_json::to_vec(value)?.len() <= 16 * 1024,
            "schema 超过 16 KiB"
        );
        ensure!(
            value
                .get("$schema")
                .is_none_or(|draft| draft == "https://json-schema.org/draft/2020-12/schema"),
            "schema 必须使用 JSON Schema 2020-12"
        );
        let mut remaining = vec![(value, 0)];
        let mut nodes = 0;
        while let Some((value, depth)) = remaining.pop() {
            nodes += 1;
            ensure!(depth <= 32 && nodes <= 2048, "schema 结构超过配额");
            match value {
                Value::Object(fields) => {
                    remaining.extend(fields.values().map(|value| (value, depth + 1)))
                }
                Value::Array(items) => {
                    remaining.extend(items.iter().map(|value| (value, depth + 1)))
                }
                _ => {}
            }
        }
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .offline()
            .with_pattern_options(
                jsonschema::PatternOptions::fancy_regex()
                    .backtrack_limit(10000)
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(1024 * 1024),
            )
            .build(value)
            .map_err(|error| anyhow::anyhow!("schema 无效: {error}"))?;
        Ok(Self(Some(validator)))
    }

    pub fn is_valid(&self, value: &Value) -> bool {
        self.0
            .as_ref()
            .is_none_or(|validator| validator.is_valid(value))
    }

    /// 错误只说明契约边界，不携带业务输入或输出。
    pub fn check(&self, value: &Value, direction: &str) -> Result<()> {
        ensure!(self.is_valid(value), "{direction}不符合 schema");
        Ok(())
    }
}
