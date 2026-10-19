use super::*;
use crate::{
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, request_context},
};
use anyhow::{Result, ensure};
use axum::http::HeaderMap;
use az_plugin_bundle::Bundle;
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

    async fn member_active(&self, _: &str, user: &str) -> Result<bool> {
        Ok(user != "removed")
    }
    async fn session_active(&self, _: &str, _: &str, user: &str) -> Result<bool> {
        Ok(user != "removed")
    }
}

#[async_trait::async_trait]
impl crate::authorization::AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和已构建 WIT Component"]
async fn tenant_installation_grants_members_and_menu_visibility_preserves_runtime() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.path() == "/component_market_test",
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let identity = Arc::new(Identity);
    let state =
        RuntimeState::isolated_component_test(identity.clone(), &database, &base, root.path())
            .await?;
    let components = state.components()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/access-{tenant}.git");
    let package_root = root.path().join("package");
    let first = super::tests::package(&package_root, &git, None, "1.0.0")?;
    let source = components.publish(first.clone(), "权限测试").await?;
    components.install(&tenant, &git, None).await?;
    let router = crate::runtime::server::routes::router(state.clone())?;
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
    let authorization = components.services.enter(&session, &source.to_string())?;
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
    assert_eq!(
        components.bundle(source, &tenant).await?.digest(),
        first.digest
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

    // 导航标签覆盖：只有管理身份能改，改写显示名而不动标识、权限或挂载入口。
    let labels = format!("{base}/api/runtime/navigation/labels");
    let page_id = format!("component:{source}:fixture");
    let before_rename: Value = bootstrap("member", &tenant).send().await?.json().await?;
    let before_page = before_rename["data"]["catalog"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["id"] == Value::String(page_id.clone()))
        .cloned()
        .unwrap();
    let rename = serde_json::json!({"kind":"page","key":page_id,"label":"成员账号"});
    assert_eq!(
        client.post(&labels).json(&rename).send().await?.status(),
        401
    );
    assert_eq!(
        client
            .post(&labels)
            .json(&rename)
            .header("x-test-user", "member")
            .header("x-test-tenant", &tenant)
            .send()
            .await?
            .status(),
        403
    );
    assert_eq!(
        client
            .post(&labels)
            .json(&serde_json::json!({"kind":"page","key":page_id,"label":"   "}))
            .header("x-test-user", "manager")
            .header("x-test-tenant", &tenant)
            .send()
            .await?
            .status(),
        400
    );
    assert_eq!(
        client
            .post(&labels)
            .json(
                &serde_json::json!({"kind":"page","key":"component:missing:page","label":"不存在"})
            )
            .header("x-test-user", "manager")
            .header("x-test-tenant", &tenant)
            .send()
            .await?
            .status(),
        404
    );
    client
        .post(&labels)
        .json(&rename)
        .header("x-test-user", "manager")
        .header("x-test-tenant", &tenant)
        .send()
        .await?
        .error_for_status()?;
    let renamed: Value = bootstrap("member", &tenant).send().await?.json().await?;
    let renamed_page = renamed["data"]["catalog"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["id"] == Value::String(page_id.clone()))
        .cloned()
        .unwrap();
    assert_eq!(renamed_page["label"], "成员账号");
    for field in ["id", "scene", "menu_path", "required_permission", "body"] {
        assert_eq!(
            renamed_page[field], before_page[field],
            "覆盖不得改写 {field}"
        );
    }
    assert_eq!(
        renamed["data"]["catalog"]["navigation_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["key"] == Value::String(page_id.clone()))
            .count(),
        1
    );
    // 其他租户是独立的导航偏好。
    let other_tenant: Value = bootstrap("manager", "other-tenant")
        .send()
        .await?
        .json()
        .await?;
    assert!(
        other_tenant["data"]["catalog"]["navigation_labels"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // 恢复默认后回到整包声明的显示名。
    client
        .delete(&labels)
        .json(&serde_json::json!({"kind":"page","key":page_id}))
        .header("x-test-user", "manager")
        .header("x-test-tenant", &tenant)
        .send()
        .await?
        .error_for_status()?;
    let restored: Value = bootstrap("member", &tenant).send().await?.json().await?;
    let restored_page = restored["data"]["catalog"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["id"] == Value::String(page_id.clone()))
        .cloned()
        .unwrap();
    assert_eq!(restored_page["label"], before_page["label"]);

    // 旧版本元数据回填不依赖角色或当前成员。
    sqlx::query("UPDATE component_versions SET permissions=NULL WHERE digest=$1")
        .bind(&first.digest)
        .execute(state.service::<sqlx::PgPool>()?.as_ref())
        .await?;
    crate::runtime::server::plugins::PostgresComponentCatalog::open(
        state.service::<sqlx::PgPool>()?.as_ref().clone(),
    )
    .await?;
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
    components.publish(second.clone(), "新版权限").await?;
    components.install(&tenant, &git, None).await?;
    assert_eq!(
        components.permissions(&tenant).await?,
        vec![services::permission(source, "fixture.edit")]
    );
    components.change(&tenant, source, "rollback").await?;
    assert_eq!(
        components.permissions(&tenant).await?,
        vec![permission.clone()]
    );
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
        if action == "enable" {
            assert_eq!(
                components.permissions(&tenant).await?,
                vec![permission.clone()]
            );
            let selected = sqlx::query_as::<_, (String, Option<String>)>(
                "SELECT digest,excluded_digest FROM component_installations WHERE source_id=$1 AND tenant_id=$2",
            )
            .bind(source)
            .bind(&tenant)
            .fetch_one(state.service::<sqlx::PgPool>()?.as_ref())
            .await?;
            assert_eq!(
                selected,
                (first.digest.clone(), Some(second.digest.clone()))
            );
        }
    }
    server.abort();
    Ok(())
}
