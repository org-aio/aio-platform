use super::*;
use crate::{
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, request_context},
};
use axum::http::HeaderMap;
use az_plugin_contract::InvocationScope;
use az_plugin_runtime::HostServices;
use serde_json::Value;

struct Identity;
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(user) = headers.get("x-test-user").and_then(|v| v.to_str().ok()) else {
            return Ok(None);
        };
        let tenant = headers.get("x-test-tenant").unwrap().to_str()?;
        Ok(Some(SessionContext {
            session_id: user.into(),
            user_id: user.into(),
            account: user.into(),
            display_name: user.into(),
            tenant_id: tenant.into(),
            tenant_label: tenant.into(),
            permissions: if user == "manager" {
                vec!["plugin:manage".into()]
            } else {
                vec![
                    "workspace:view".into(),
                    "component:obsolete:fixture.view".into(),
                ]
            },
        }))
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, _: &str, user: &str) -> Result<bool> {
        Ok(user != "removed")
    }
    async fn session_active(&self, _: &str, _: &str, user: &str) -> Result<bool> {
        Ok(user != "removed")
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和已构建 WIT Component"]
async fn tenant_installation_grants_members_and_menu_visibility_preserves_runtime() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    ensure!(
        database.contains("localhost")
            && database
                .split('?')
                .next()
                .unwrap()
                .ends_with("/component_market_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let identity = Arc::new(Identity);
    let mut state =
        RuntimeState::isolated_admin_test(identity.clone(), &database, &base, root.path()).await?;
    let components = Components::open(
        state.store.pool.clone(),
        &database,
        &root.path().join("keys.json"),
        root.path().join("objects"),
        identity,
    )
    .await?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/access-{tenant}.git");
    let package_root = root.path().join("package");
    let first = super::tests::package(&package_root, &git, None, "1.0.0")?;
    let source = components.publish(first.clone(), "权限测试").await?;
    components.install(&tenant, &git, None).await?;
    state.components = Some(components.clone());
    let router = crate::runtime::server::routes::router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::new();
    let bootstrap = |user: &str, tenant: &str| {
        client
            .get(format!("{base}/api/runtime/bootstrap"))
            .header("x-test-user", user)
            .header("x-test-tenant", tenant)
    };
    let permission = services::permission(source, "fixture.view");
    for user in ["member", "new-member", "manager"] {
        let snapshot: Value = bootstrap(user, &tenant)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let permissions = snapshot["data"]["permissions"].as_array().unwrap();
        assert!(permissions.contains(&Value::String(permission.clone())));
        assert!(!permissions.contains(&Value::String("component:obsolete:fixture.view".into())));
        if user != "manager" {
            assert!(!permissions.contains(&Value::String("plugin:manage".into())));
        }
    }
    let other: Value = bootstrap("member", "other-tenant")
        .send()
        .await?
        .json()
        .await?;
    assert!(
        !other["data"]["permissions"]
            .as_array()
            .unwrap()
            .contains(&Value::String(permission.clone()))
    );
    let removed: Value = bootstrap("removed", &tenant).send().await?.json().await?;
    assert!(removed["data"].is_null());

    let mut headers = HeaderMap::new();
    headers.insert("x-test-user", "member".parse()?);
    headers.insert("x-test-tenant", tenant.parse()?);
    let session = request_context::authenticate(&state, &headers)
        .await
        .map_err(|_| anyhow::anyhow!("测试成员鉴权失败"))?;
    let authorization = components.services.enter(&session)?;
    let scope = InvocationScope {
        source_id: source.to_string(),
        revision: first.digest.clone(),
        context: authorization.context.clone(),
        grants: Default::default(),
    };
    assert!(
        components
            .services
            .authorize(&scope, "fixture.view")
            .await?
    );
    assert!(
        !components
            .services
            .authorize(&scope, "plugin:manage")
            .await?
    );

    let before = components.description(&tenant, source).await?;
    let hide = format!("{base}/api/runtime/plugins/{source}/hide-menu");
    assert_eq!(client.post(&hide).send().await?.status(), 401);
    assert_eq!(
        client
            .post(&hide)
            .header("x-test-user", "member")
            .header("x-test-tenant", &tenant)
            .send()
            .await?
            .status(),
        403
    );
    assert_eq!(
        client
            .post(&hide)
            .header("x-test-user", "manager")
            .header("x-test-tenant", "other-tenant")
            .send()
            .await?
            .status(),
        403
    );
    let old = bootstrap("member", &tenant).send().await?;
    let etag = old.headers()["etag"].clone();
    client
        .post(&hide)
        .header("x-test-user", "manager")
        .header("x-test-tenant", &tenant)
        .send()
        .await?
        .error_for_status()?;
    let hidden = bootstrap("member", &tenant)
        .header("if-none-match", etag)
        .send()
        .await?;
    assert_eq!(hidden.status(), 200);
    let hidden: Value = hidden.json().await?;
    assert!(
        !hidden["data"]["catalog"]["hidden_pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !hidden["data"]["catalog"]["pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(components.description(&tenant, source).await?.1, before.1);
    assert!(
        components
            .slot(source, &tenant)
            .await?
            .snapshot()
            .await?
            .is_some()
    );
    assert_eq!(
        components.permissions(&tenant).await?,
        vec![permission.clone()]
    );
    let market: Value = client
        .get(format!("{base}/api/runtime/marketplace"))
        .header("x-test-user", "manager")
        .header("x-test-tenant", &tenant)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        market["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["git"] == git)
            .unwrap()["menu_hidden"],
        true
    );
    client
        .post(format!("{base}/api/runtime/plugins/{source}/show-menu"))
        .header("x-test-user", "manager")
        .header("x-test-tenant", &tenant)
        .send()
        .await?
        .error_for_status()?;
    let visible: Value = bootstrap("member", &tenant).send().await?.json().await?;
    assert!(
        visible["data"]["catalog"]["hidden_pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // 旧版本元数据回填不依赖角色或当前成员。
    sqlx::query("UPDATE component_versions SET permissions=NULL WHERE digest=$1")
        .bind(&first.digest)
        .execute(&components.pool)
        .await?;
    super::permissions::backfill(&components.pool).await?;
    assert_eq!(
        components.permissions(&tenant).await?,
        vec![permission.clone()]
    );
    let manifest = package_root.join("aio-plugin.toml");
    std::fs::write(
        &manifest,
        std::fs::read_to_string(&manifest)?.replace("fixture.view", "fixture.edit"),
    )?;
    let second = Bundle::from_directory(
        &package_root,
        "aio-plugin.toml",
        git.clone(),
        "b".repeat(40),
        "2.0.0".into(),
    )?;
    components.publish(second, "新版权限").await?;
    components.install(&tenant, &git, None).await?;
    assert_eq!(
        components.permissions(&tenant).await?,
        vec![services::permission(source, "fixture.edit")]
    );
    components.change(&tenant, source, "rollback").await?;
    assert_eq!(components.permissions(&tenant).await?, vec![permission]);
    for action in ["disable", "enable", "uninstall"] {
        components.change(&tenant, source, action).await?;
        let current = request_context::authenticate(&state, &headers)
            .await
            .map_err(|_| anyhow::anyhow!("测试成员鉴权失败"))?;
        assert_eq!(
            current
                .permissions
                .iter()
                .any(|p| p.starts_with("component:")),
            action == "enable"
        );
    }
    server.abort();
    Ok(())
}
