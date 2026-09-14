use super::*;
use std::fs;

#[derive(Default)]
struct TestIdentity(std::sync::Mutex<Vec<String>>);
#[async_trait::async_trait]
impl crate::identity::IdentityProvider for TestIdentity {
    async fn can_publish(&self, _: &crate::identity::SessionContext) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn authenticate(
        &self,
        _: &axum::http::HeaderMap,
    ) -> Result<Option<crate::identity::SessionContext>> {
        Ok(None)
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn install_permissions(&self, _: &str, permissions: &[String]) -> Result<()> {
        self.0.lock().unwrap().extend_from_slice(permissions);
        Ok(())
    }
}

fn package(root: &Path, git: &str, parent: Option<&str>, version: &str) -> Result<Bundle> {
    fs::create_dir_all(root.join("frontend"))?;
    fs::write(
        root.join("frontend/index.html"),
        "<html><body>Native Component</body></html>",
    )?;
    fs::copy(
        std::env::var("AIO_TEST_HEALTHY_COMPONENT")?,
        root.join("plugin.wasm"),
    )?;
    let parent = parent
        .map(|p| format!("parent={p:?}\n"))
        .unwrap_or_default();
    fs::write(
        root.join("aio-plugin.toml"),
        format!(
            "schema_version=2\n[plugin]\npermissions=['fixture.view']\n[plugin.marketplace]\ntitle='Fixture'\nsummary='Installation verification'\nlicense='MIT'\n{parent}[plugin.runtime]\nartifact='plugin.wasm'\nhost_version='>=2026.9.11'\n[plugin.frontend]\npath='frontend'\n"
        ),
    )?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        version.into(),
    )
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和已构建 WIT Component"]
async fn published_components_install_optionally_and_restore() -> Result<()> {
    let url = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    ensure!(
        (url.contains("localhost") || url.contains("127.0.0.1"))
            && url.contains("component_market_test"),
        "只接受本机独立市场测试库"
    );
    let pool = PgPool::connect(&url).await?;
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS plugin_sources(id TEXT PRIMARY KEY,git TEXT); CREATE TABLE IF NOT EXISTS tenant_plugin_bindings(tenant_id TEXT,source_id TEXT,enabled BOOLEAN);").execute(&pool).await?;
    let temp = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let identity = Arc::new(TestIdentity::default());
    let git = format!("https://github.com/example/parent-{tenant}.git");
    let child_git = format!("https://github.com/example/child-{tenant}.git");
    let first = package(&temp.path().join("parent"), &git, None, "1.0.0")?;
    let child = package(&temp.path().join("child"), &child_git, Some(&git), "1.0.0")?;
    let components = Components::open(
        pool.clone(),
        &url,
        &temp.path().join("keys.json"),
        temp.path().join("objects"),
        identity.clone(),
    )
    .await?;
    let parent = components.publish(first.clone(), "Parent").await?;
    let child_id = components.publish(child.clone(), "Child").await?;
    assert!(
        components
            .entries(&tenant)
            .await?
            .iter()
            .all(|e| !e.installed)
    );
    assert!(components.install(&tenant, &child_git, None).await.is_err());
    components.install(&tenant, &git, None).await?;
    let grants = identity.0.lock().unwrap().clone();
    assert!(grants.contains(&services::permission(parent, "fixture.view")));
    assert!(!grants.contains(&"fixture.view".to_owned()));
    assert!(
        !components
            .entries(&tenant)
            .await?
            .into_iter()
            .find(|e| e.git == child_git)
            .unwrap()
            .installed
    );
    assert!(
        components
            .install("other-tenant", &child_git, None)
            .await
            .is_err()
    );
    components.install(&tenant, &child_git, None).await?;
    assert!(
        components
            .change(&tenant, parent, "uninstall")
            .await
            .is_err()
    );
    assert!(components.change(&tenant, parent, "disable").await.is_err());
    let second = package(&temp.path().join("parent"), &git, None, "2.0.0")?;
    components.publish(second.clone(), "Second").await?;
    assert_eq!(
        components
            .slot(parent, &tenant)
            .await?
            .stored()
            .await?
            .unwrap()
            .bundle
            .digest,
        first.digest
    );
    components.install(&tenant, &git, None).await?;
    let mut invalid = second.clone();
    invalid.digest = "0".repeat(64);
    assert!(components.publish(invalid, "").await.is_err());
    assert_eq!(
        components
            .slot(parent, &tenant)
            .await?
            .stored()
            .await?
            .unwrap()
            .bundle
            .digest,
        second.digest
    );
    components.change(&tenant, parent, "rollback").await?;
    assert_eq!(
        components
            .slot(parent, &tenant)
            .await?
            .stored()
            .await?
            .unwrap()
            .bundle
            .digest,
        first.digest
    );
    components.slot(parent, &tenant).await?.deactivate().await?;
    for slot in components.slots.lock().await.values() {
        slot.unload().await;
    }
    drop(components);
    let restored = Components::open(
        pool,
        &url,
        &temp.path().join("keys.json"),
        temp.path().join("objects"),
        identity.clone(),
    )
    .await?;
    restored.restore().await?;
    assert_eq!(
        restored
            .slot(parent, &tenant)
            .await?
            .snapshot()
            .await?
            .unwrap()
            .bundle
            .digest(),
        first.digest
    );
    restored.change(&tenant, child_id, "uninstall").await?;
    restored.change(&tenant, parent, "uninstall").await?;
    assert!(
        restored
            .entries(&tenant)
            .await?
            .iter()
            .all(|e| !e.installed)
    );
    Ok(())
}

#[test]
fn binary_requests_reject_forged_identity_and_escaping_paths() {
    let request = |path: &str, headers: serde_json::Value| {
        serde_json::from_value::<model::Request>(
            serde_json::json!({"method":"GET","path":path,"body":[],"headers":headers}),
        )
        .unwrap()
    };
    use az_plugin_runtime::bindings::aio::plugin::transport::Request;
    assert!(Request::try_from(request("/screens", serde_json::json!([]))).is_ok());
    assert!(Request::try_from(request("/../secrets", serde_json::json!([]))).is_err());
    assert!(
        Request::try_from(request(
            "/screens",
            serde_json::json!([{"name":"x-aio-user-id","value":"admin"}])
        ))
        .is_err()
    );
}
