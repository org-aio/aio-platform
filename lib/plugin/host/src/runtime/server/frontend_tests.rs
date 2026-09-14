use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use crate::identity::SessionContext;
use axum::http::HeaderValue;

use super::{
    frontend_access::{FrontendAccess, FrontendGrant},
    frontend_document,
};

pub(super) fn grant() -> FrontendGrant {
    FrontendGrant {
        cookie: HeaderValue::from_static("session=testing"),
        session_id: "session-a".to_owned(),
        tenant_id: "tenant-a".to_owned(),
        user_id: "user-a".to_owned(),
        page_id: "counter".to_owned(),
        source_id: "source-a".to_owned(),
        activation_generation: "activation-a".to_owned(),
        revision: "a".repeat(64),
        entry: "index.html".to_owned(),
        frontend_path: "dist/web".to_owned(),
        assets: BTreeMap::new(),
        issued: Instant::now(),
    }
}

pub(super) fn session() -> SessionContext {
    SessionContext {
        session_id: "session-a".to_owned(),
        user_id: "user-a".to_owned(),
        tenant_id: "tenant-a".to_owned(),
        account: "test".to_owned(),
        display_name: "Test".to_owned(),
        tenant_label: "Test".to_owned(),
        permissions: Vec::new(),
    }
}

#[test]
fn grants_expire_revoke_and_enforce_per_user_quota() -> anyhow::Result<()> {
    let access = FrontendAccess::new("http://127.0.0.1:8080")?;
    let token = access.issue(grant())?;
    assert_eq!(access.get(&token)?.page_id, "counter");
    access.remove(&token)?;
    assert!(access.get(&token).is_err());
    let mut expired = grant();
    expired.issued = Instant::now() - Duration::from_secs(1801);
    let expired_token = access.issue(expired)?;
    assert!(access.get(&expired_token).is_err());
    for _ in 0..16 {
        access.issue(grant())?;
    }
    assert!(access.issue(grant()).is_err());
    Ok(())
}

#[test]
fn renew_keeps_token_without_reviving_expired_or_removed_mounts() -> anyhow::Result<()> {
    let access = FrontendAccess::new("http://127.0.0.1:8080")?;
    let mut old = grant();
    old.issued = Instant::now() - Duration::from_secs(1790);
    let token = access.issue(old)?;
    access.renew(&token)?;
    assert!(access.get(&token)?.issued.elapsed() < Duration::from_secs(1));
    access.remove(&token)?;
    assert!(access.renew(&token).is_err());
    let mut old = grant();
    old.issued = Instant::now() - Duration::from_secs(1801);
    let token = access.issue(old)?;
    assert!(access.renew(&token).is_err());
    Ok(())
}

#[test]
fn document_bootstrap_replaces_base_and_stays_in_sandbox() -> anyhow::Result<()> {
    let prefix = "https://aio.example/api/runtime/frontend/assets/token/";
    let rendered = frontend_document::render_entry(b"<!doctype html><html><head><base href='https://evil.example/'></head><body><script type='module' src='assets/app.js'></script></body></html>", prefix, "index.html", "token", false)?;
    let rendered = String::from_utf8(rendered)?;
    assert_eq!(rendered.matches("<base").count(), 1);
    assert!(rendered.contains(&format!("href=\"{prefix}\"")));
    assert!(rendered.contains("__aio_bridge.js"));
    assert!(rendered.contains("__aio_modules.js"));
    assert!(rendered.contains("type=\"module-shim\""));
    assert!(!rendered.contains("evil.example"));
    let policy = frontend_document::content_policy(prefix);
    assert!(policy.contains("sandbox allow-scripts;"));
    assert!(policy.contains("script-src 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'"));
    assert!(!policy.contains("allow-same-origin"));
    assert!(
        policy
            .contains("connect-src blob: https://aio.example/api/runtime/frontend/assets/token/;")
    );
    assert!(FrontendAccess::new("https://aio.example/path").is_err());
    assert!(FrontendAccess::new("http://public.example").is_err());
    let nested = String::from_utf8(frontend_document::render_entry(
        b"<head></head>",
        prefix,
        "pages/index.html",
        "token",
        false,
    )?)?;
    assert!(nested.contains(&format!("href=\"{prefix}pages/\"")));
    Ok(())
}
