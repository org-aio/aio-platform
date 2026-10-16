use anyhow::{Result, ensure};
use serde_json::Value;

/// 发布时编译，调用时复用；不访问文件或网络，不把业务输入写入错误消息。
pub(crate) fn compile(schema: Option<&schemars::Schema>) -> Result<Option<jsonschema::Validator>> {
    let Some(schema) = schema else {
        return Ok(None);
    };
    let value = schema.as_value();
    ensure!(
        serde_json::to_vec(value)?.len() <= 16 * 1024,
        "增强 schema 超过 16 KiB"
    );
    ensure!(
        value
            .get("$schema")
            .is_none_or(|draft| draft == "https://json-schema.org/draft/2020-12/schema"),
        "增强 schema 必须使用 JSON Schema 2020-12"
    );
    let mut remaining = vec![(value, 0)];
    let mut nodes = 0;
    while let Some((value, depth)) = remaining.pop() {
        nodes += 1;
        ensure!(depth <= 32 && nodes <= 2048, "增强 schema 结构超过配额");
        match value {
            Value::Object(fields) => {
                remaining.extend(fields.values().map(|value| (value, depth + 1)))
            }
            Value::Array(items) => remaining.extend(items.iter().map(|value| (value, depth + 1))),
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
        .map_err(|error| anyhow::anyhow!("增强 schema 无效: {error}"))?;
    Ok(Some(validator))
}

pub(crate) fn check(
    validator: &Option<jsonschema::Validator>,
    value: &Value,
    direction: &str,
) -> Result<()> {
    ensure!(
        validator
            .as_ref()
            .is_none_or(|validator| validator.is_valid(value)),
        "增强{direction}不符合 schema"
    );
    Ok(())
}

pub(crate) fn same(left: &Option<schemars::Schema>, right: &Option<schemars::Schema>) -> bool {
    let any = Value::Bool(true);
    left.as_ref()
        .map(schemars::Schema::as_value)
        .unwrap_or(&any)
        == right
            .as_ref()
            .map(schemars::Schema::as_value)
            .unwrap_or(&any)
}
