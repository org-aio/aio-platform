use crate::{Bundle, BundleManifest};
use base64::{Engine, engine::general_purpose::STANDARD};

fn manifest(extra: &str) -> String {
    format!(
        r#"schema_version = 2
[plugin.runtime]
artifact = "dist/server"
host_version = ">=2026.9.11"
[plugin.frontend]
path = "dist/frontend"
[plugin.runtime.process]
image = "node:22@sha256:{}"
{extra}
"#,
        "a".repeat(64)
    )
}

#[test]
fn process_requires_immutable_image_and_explicit_https_services() {
    assert!(BundleManifest::parse(&manifest("endpoints = ['https://api.openai.com/v1']\nservices = ['https://github.com/zjarlin/aio-plugin-agent-memory.git']")).is_ok());
    assert!(
        BundleManifest::parse(&manifest("endpoints = ['http://192.168.31.252:18080/v1']")).is_ok()
    );
    for extra in [
        "endpoints = ['http://127.0.0.1']",
        "endpoints = ['http://169.254.169.254/latest']",
        "endpoints = ['http://example.com/v1']",
        "endpoints = ['https://user:secret@host/v1']",
        "endpoints = ['https://host/v1?key=value']",
        "services = ['file:///tmp/service']",
    ] {
        assert!(BundleManifest::parse(&manifest(extra)).is_err());
    }
    assert!(
        BundleManifest::parse(&manifest("").replace(&format!("@sha256:{}", "a".repeat(64)), ""))
            .is_err()
    );
}

#[test]
fn process_accepts_workspace_execution_without_opening_arbitrary_capabilities() {
    assert!(
        BundleManifest::parse(&manifest(
            "worker_capabilities = ['desktop.open-app', 'desktop.control', 'skills.sync', 'workspace.execute']"
        ))
        .is_ok()
    );
    for extra in [
        "worker_capabilities = ['shell.execute']",
        "worker_capabilities = ['*']",
        "worker_capabilities = ['desktop.open-app', 'skills.sync', 'workspace.execute', 'shell.execute']",
    ] {
        assert!(BundleManifest::parse(&manifest(extra)).is_err());
    }
}

#[test]
fn process_accepts_subplugin_route_declarations() {
    let manifest = manifest("[[plugin.subplugins]]\nid = 'boxun'\nroutes = ['admin-api']\n");
    let parsed = BundleManifest::parse(&manifest).expect("子插件声明必须被进程包接受");
    assert_eq!(parsed.plugin.subplugins.len(), 1);
    assert_eq!(parsed.plugin.subplugins[0].id, "boxun");
    assert_eq!(parsed.plugin.subplugins[0].routes, ["admin-api"]);
}

#[test]
fn process_archive_binds_executable_and_rejects_other_platforms() -> anyhow::Result<()> {
    let mut binary = vec![0; 64];
    binary[..6].copy_from_slice(b"\x7fELF\x02\x01");
    binary[18] = 62;
    let mut bundle = Bundle {
        abi_version: 2,
        git: "https://github.com/example/agent.git".into(),
        commit: "a".repeat(40),
        version: "0.1.0".into(),
        manifest: manifest(""),
        files: [
            ("dist/server".into(), STANDARD.encode(&binary)),
            (
                "dist/frontend/index.html".into(),
                STANDARD.encode(b"<html></html>"),
            ),
        ]
        .into(),
        digest: String::new(),
    };
    bundle.digest = bundle.content_digest();
    assert_eq!(
        Bundle::decode(&bundle.encode()?)?.verify()?.component(),
        binary
    );
    binary[18] = 183;
    bundle
        .files
        .insert("dist/server".into(), STANDARD.encode(binary));
    bundle.digest = bundle.content_digest();
    assert!(bundle.verify().is_err());
    Ok(())
}
