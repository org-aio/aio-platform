use super::*;

fn request(method: &str, path: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        query: None,
        headers: vec![],
        body: vec![],
    }
}

#[test]
fn management_contract_excludes_identity_internal_services_and_encoded_paths() {
    for (method, path) in [
        ("GET", "/api/runtime/catalog"),
        ("POST", "/api/runtime/plugins/install"),
        (
            "POST",
            "/api/runtime/plugins/00000000-0000-0000-0000-000000000001/disable",
        ),
        ("GET", "/api/runtime/tools/access"),
        ("GET", "/api/runtime/workers/tasks"),
    ] {
        assert!(validate(&request(method, path)).is_ok(), "{path}");
    }
    for (method, path) in [
        ("GET", "https://outside/api/runtime/catalog"),
        ("GET", "//api/runtime/catalog"),
        ("GET", "/api/runtime/marketplace/../catalog"),
        ("GET", "/api/runtime/marketplace/%2e%2e/catalog"),
        ("GET", "/api/runtime/services/plugin/path"),
        ("GET", "/api/identity/me"),
        ("POST", "/api/internal/delivery/claim"),
        ("POST", "/api/runtime/workers/tasks"),
    ] {
        assert!(validate(&request(method, path)).is_err(), "{path}");
    }
    for name in [
        "cookie",
        "authorization",
        "x-aio-user-id",
        "x-aio-context",
        "host",
    ] {
        let mut forged = request("GET", "/api/runtime/catalog");
        forged.headers.push(Header {
            name: name.into(),
            value: "forged".into(),
        });
        assert!(validate(&forged).is_err());
    }
}
