use anyhow::Result;
use serde_json::json;

#[test]
fn schema_compiler_accepts_local_references_and_refuses_external_resources_and_excessive_structure()
-> Result<()> {
    let schema = json!({"$defs":{"count":{"type":"integer","minimum":1}},"type":"object","properties":{"count":{"$ref":"#/$defs/count"}},"required":["count"],"additionalProperties":false}).try_into()?;
    let compiled = super::ValueSchema::compile(Some(&schema))?;
    compiled.check(&json!({"count":2}), "输入")?;
    assert!(compiled.check(&json!({"count":0}), "输入").is_err());
    for schema in [
        json!({"$ref":"https://127.0.0.1:1/private"}),
        json!({"$ref":"file:///etc/passwd"}),
        json!({"type":"unknown"}),
        json!({"$schema":"http://json-schema.org/draft-07/schema#"}),
        json!({"description":"x".repeat(16385)}),
        json!({"enum": vec![true; 2050]}),
    ] {
        assert!(super::ValueSchema::compile(Some(&schema.try_into()?)).is_err());
    }
    let mut deep = json!({});
    for _ in 0..34 {
        deep = json!({"items":deep});
    }
    assert!(super::ValueSchema::compile(Some(&deep.try_into()?)).is_err());
    Ok(())
}
