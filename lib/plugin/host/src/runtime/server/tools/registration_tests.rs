use super::*;
use crate::identity::{IdentityProvider, SessionContext};
use anyhow::Result;
use axum::http::HeaderMap;
use az_tool::registration::{Metadata, Registration};
use std::sync::Arc;

struct Identity;
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(role) = headers.get("x-test-role").and_then(|h| h.to_str().ok()) else {
            return Ok(None);
        };
        Ok(Some(SessionContext {
            session_id: "test".into(),
            user_id: role.into(),
            account: role.into(),
            display_name: role.into(),
            tenant_id: "test".into(),
            tenant_label: "test".into(),
            permissions: vec!["plugin:manage".into()],
        }))
    }
    async fn can_publish(&self, session: &SessionContext) -> Result<bool> {
        Ok(session.account == "publisher")
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn install_permissions(&self, _: &str, _: &[String]) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn registration_http_auth_persistence_metadata_and_no_server_execution() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let root = tempfile::tempdir()?;
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        &database,
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server =
        tokio::spawn(axum::serve(listener, router().with_state(state.clone())).into_future());
    let client = reqwest::Client::new();
    let marker = root.path().join("must-not-execute");
    let request = Registration {
        command: format!("touch {}", marker.display()),
        platforms: vec!["macos".into()],
        ..Default::default()
    };
    let url = format!("{base}/api/runtime/tools/register");
    assert_eq!(client.post(&url).json(&request).send().await?.status(), 401);
    assert_eq!(
        client
            .post(&url)
            .header("x-test-role", "manager")
            .json(&request)
            .send()
            .await?
            .status(),
        403
    );
    let value: serde_json::Value = client
        .post(&url)
        .header("x-test-role", "publisher")
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let manifest: az_tool::ToolManifest = serde_json::from_value(value["data"].clone())?;
    assert!(!marker.exists());
    let original = manifest.platforms.clone();
    let metadata = Metadata {
        title: "编辑后的标题".into(),
        summary: "编辑后的备注".into(),
        ..Default::default()
    };
    let details = format!("{base}/api/runtime/tools/{}/details", manifest.id);
    assert_eq!(
        client
            .patch(&details)
            .header("x-test-role", "manager")
            .json(&metadata)
            .send()
            .await?
            .status(),
        403
    );
    client
        .patch(&details)
        .header("x-test-role", "publisher")
        .json(&metadata)
        .send()
        .await?
        .error_for_status()?;
    let public: az_tool::ToolManifest = client
        .get(format!("{base}/api/runtime/tools/{}/1.0.0", manifest.id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(public.title, metadata.title);
    assert_eq!(public.platforms, original);
    let duplicate: serde_json::Value = client
        .post(&url)
        .header("x-test-role", "publisher")
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(duplicate["data"]["id"], manifest.id);
    assert_eq!(duplicate["data"]["title"], metadata.title);
    assert!(
        storage::documentation(&state.store.pool, &manifest.id)
            .await?
            .is_some()
    );
    assert!(!marker.exists());
    server.abort();
    sqlx::query("DELETE FROM marketplace_tool_details WHERE id=$1")
        .bind(&manifest.id)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("DELETE FROM marketplace_tools WHERE id=$1")
        .bind(&manifest.id)
        .execute(&state.store.pool)
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要公网 Git，设置 AIO_TEST_README_GIT"]
async fn public_git_readme_is_loaded_without_running_install_command() -> Result<()> {
    let root = tempfile::tempdir()?;
    let git = std::env::var("AIO_TEST_README_GIT")?;
    if let Ok(address) = std::env::var("AIO_TEST_README_ADDRESS") {
        let remote = super::super::remote_access::RemoteResolution {
            host: "github.com".into(),
            socket: format!("{address}:443").parse()?,
        };
        let (readme, revision) =
            documents::read_remote(root.path(), &format!("{git}.git"), &remote).await?;
        assert!(!readme.is_empty());
        assert_eq!(revision.len(), 40);
        return Ok(());
    }
    let doc = documents::load(
        root.path(),
        Metadata {
            git,
            ..Default::default()
        },
    )
    .await;
    assert!(doc.error.is_none(), "{:?}", doc.error);
    assert!(!doc.readme.is_empty());
    assert!(doc.image_base.starts_with("https://"));
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    Ok(())
}

#[tokio::test]
async fn private_git_address_is_not_contacted_and_failure_is_reported() -> Result<()> {
    let root = tempfile::tempdir()?;
    let doc = documents::load(
        root.path(),
        Metadata {
            git: "https://127.0.0.1/internal".into(),
            ..Default::default()
        },
    )
    .await;
    assert!(doc.error.unwrap().contains("非公网"));
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    Ok(())
}
