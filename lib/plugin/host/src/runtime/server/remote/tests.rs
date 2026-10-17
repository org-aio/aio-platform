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
    let validator = super::bindings::validator(&local)?;
    assert!(validator.is_valid(&json!(7)));
    assert!(!validator.is_valid(&json!("seven")));
    for reference in ["https://example.com/schema.json", "file:///etc/passwd"] {
        let schema = serde_json::from_value(json!({"$ref":reference}))?;
        assert!(super::bindings::validator(&schema).is_err());
    }
    Ok(())
}

#[cfg(not(all(feature = "remote-mcp", feature = "remote-grpc")))]
#[test]
fn uncompiled_transports_are_rejected_while_decoding_configuration() -> Result<()> {
    #[cfg(not(feature = "remote-grpc"))]
    {
        let result = serde_json::from_value::<super::configuration::Transport>(json!({
            "protocol": "grpc", "descriptor_set": "/must-not-read.pb", "service": "Test", "method": "Run"
        }));
        let error = result
            .err()
            .ok_or_else(|| anyhow::anyhow!("接受了未编译的 gRPC"))?;
        assert!(error.to_string().contains("unknown variant `grpc`"));
    }
    #[cfg(not(feature = "remote-mcp"))]
    for protocol in ["mcp", "mcp_stdio"] {
        let result = serde_json::from_value::<super::configuration::Transport>(json!({
            "protocol": protocol, "tool": "test", "output": "structured"
        }));
        let error = result
            .err()
            .ok_or_else(|| anyhow::anyhow!("接受了未编译的 MCP"))?;
        assert!(
            error
                .to_string()
                .contains(&format!("unknown variant `{protocol}`"))
        );
    }
    Ok(())
}
