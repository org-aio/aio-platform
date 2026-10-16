use anyhow::Result;
use serde_json::json;

#[test]
fn remote_endpoints_require_explicit_secure_addresses() -> Result<()> {
    for endpoint in [
        "https://example.com/mcp",
        "http://127.0.0.1:1234/mcp",
        "http://[::1]:1234/mcp",
    ] {
        super::configuration::endpoint(endpoint, false)?;
    }
    for endpoint in [
        "http://example.com/mcp",
        "http://169.254.169.254/mcp",
        "https://key:secret@example.com/mcp",
        "https://example.com/mcp?key=secret",
    ] {
        assert!(super::configuration::endpoint(endpoint, false).is_err());
    }
    assert!(super::configuration::endpoint("https://example.com/prefix", true).is_err());
    super::configuration::endpoint("https://example.com", true)?;
    Ok(())
}

#[test]
fn schemas_only_resolve_declared_local_references() -> Result<()> {
    let local =
        serde_json::from_value(json!({"$defs":{"n":{"type":"integer"}},"$ref":"#/$defs/n"}))?;
    let validator = super::provider::validator(&local)?;
    assert!(validator.is_valid(&json!(7)));
    assert!(!validator.is_valid(&json!("seven")));
    for reference in ["https://example.com/schema.json", "file:///etc/passwd"] {
        let schema = serde_json::from_value(json!({"$ref":reference}))?;
        assert!(super::provider::validator(&schema).is_err());
    }
    Ok(())
}
