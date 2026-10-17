use rmcp::ErrorData;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// 增强接收任意 JSON，MCP 则要求对象参数；嵌入资源必须有独立 base，
// 否则业务 schema 的 #/$defs 引用会错误地解析到 MCP 外层对象。
pub(super) fn envelope(
    field: &str,
    schema: schemars::Schema,
    include_revision: bool,
) -> Result<serde_json::Map<String, Value>, ErrorData> {
    let mut value = serde_json::to_value(schema)
        .map_err(|_| ErrorData::internal_error("编码增强 schema 失败", None))?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| ErrorData::internal_error("编码增强 schema 失败", None))?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    let mut base = reqwest::Url::parse(&format!(
        "https://schemas.aio.invalid/enhancements/{digest}/{field}"
    ))
    .map_err(|_| ErrorData::internal_error("增强 schema 地址无效", None))?;
    if let Value::Object(object) = &mut value {
        if let Some(id) = object
            .get("$id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && *id != "#")
        {
            let mut resolved = base
                .join(id)
                .map_err(|_| ErrorData::internal_error("增强 schema 标识无效", None))?;
            resolved.set_fragment(None);
            if resolved == base {
                // 保留业务 id 及指向它的相对引用，只移动同目录中的外层地址。
                base.set_path(&format!("{}.mcp.json", base.path()));
            }
        } else {
            // 缺失或空 id 不能建立子资源；为局部片段引用补充独立地址。
            object.insert("$id".into(), json!(format!("{field}.schema.json")));
        }
    }
    let mut properties = serde_json::Map::new();
    properties.insert(field.into(), value);
    let mut required = vec![field];
    if include_revision {
        properties.insert("revision".into(), json!({"type":"string"}));
        required.push("revision");
    }
    serde_json::from_value(json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":base.as_str(),
        "type":"object", "properties":properties, "required":required, "additionalProperties":false
    }))
    .map_err(|_| ErrorData::internal_error("增强参数契约无效", None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_ids_keep_local_references() -> anyhow::Result<()> {
        for id in [
            None,
            Some(""),
            Some("#"),
            Some("input"),
            Some("relative.json"),
            Some("https://example.test/input"),
        ] {
            let mut references = vec!["#/$defs/value".to_owned()];
            if let Some(id) = id.filter(|id| !id.is_empty() && *id != "#") {
                references.push(format!("{id}#/$defs/value"));
            }
            for reference in references {
                let mut schema = json!({"$defs":{"value":{"type":"integer"}},"$ref":reference});
                if let Some(id) = id {
                    schema["$id"] = json!(id);
                }
                let original = jsonschema::options().offline().build(&schema)?;
                assert!(original.is_valid(&json!(3)));
                let wrapped = envelope("input", schema.try_into()?, false)?;
                let validator = jsonschema::options().offline().build(&json!(wrapped))?;
                assert!(validator.is_valid(&json!({"input":3})), "schema id: {id:?}");
                assert!(
                    !validator.is_valid(&json!({"input":"wrong"})),
                    "schema id: {id:?}"
                );
            }
        }
        Ok(())
    }
}
