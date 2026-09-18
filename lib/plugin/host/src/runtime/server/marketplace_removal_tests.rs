use super::*;
use crate::identity::{IdentityProvider, SessionContext};
use anyhow::Result;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;
use std::sync::Arc;

struct Identity;

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(role) = headers
            .get("x-test-role")
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(None);
        };
        Ok(Some(SessionContext {
            session_id: "test".into(),
            user_id: role.into(),
            account: role.into(),
            display_name: role.into(),
            tenant_id: "test".into(),
            tenant_label: "test".into(),
            permissions: if role == "member" {
                vec![]
            } else {
                vec!["plugin:manage".into()]
            },
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
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn removal_http_authorization_not_found_and_install_rejection() -> Result<()> {
    let database = std::env::var("AIO_TEST_DATABASE_URL")?;
    let root = tempfile::tempdir()?;
    let state = RuntimeState::isolated_admin_test(
        Arc::new(Identity),
        &database,
        "http://127.0.0.1:1",
        root.path(),
    )
    .await?;
    let git = format!("https://example.com/{}.git", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO marketplace_entries(source,git,rev,title,summary,license,tags,capabilities) VALUES('aio://published',$1,'old','Removal test','','MIT','[]','{}')")
        .bind(&git).execute(&state.store.pool).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(
        axum::serve(listener, super::super::routes::router(state.clone())).into_future(),
    );
    let result = exercise_http(&reqwest::Client::new(), &base, &git).await;
    server.abort();
    sqlx::query("DELETE FROM marketplace_plugin_removals WHERE git=$1")
        .bind(&git)
        .execute(&state.store.pool)
        .await?;
    sqlx::query("DELETE FROM marketplace_entries WHERE git=$1")
        .bind(&git)
        .execute(&state.store.pool)
        .await?;
    result
}

async fn exercise_http(client: &reqwest::Client, base: &str, git: &str) -> Result<()> {
    let url = format!("{base}/api/runtime/marketplace");
    let body = serde_json::json!({"git": git});
    assert_eq!(client.delete(&url).json(&body).send().await?.status(), 401);
    for role in ["member", "manager"] {
        assert_eq!(
            client
                .delete(&url)
                .header("x-test-role", role)
                .json(&body)
                .send()
                .await?
                .status(),
            403
        );
    }
    assert_eq!(
        client
            .delete(&url)
            .header("x-test-role", "publisher")
            .json(&serde_json::json!({"git":"missing"}))
            .send()
            .await?
            .status(),
        404
    );
    for _ in 0..2 {
        client
            .delete(&url)
            .header("x-test-role", "publisher")
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
    }
    let listed: serde_json::Value = client
        .get(&url)
        .header("x-test-role", "member")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(
        listed["data"]
            .as_array()
            .is_some_and(|entries| entries.iter().all(|entry| entry["git"] != git))
    );
    let install = client
        .post(format!("{base}/api/runtime/plugins/install"))
        .header("x-test-role", "manager")
        .json(&serde_json::json!({"git":git,"rev":"a".repeat(64)}))
        .send()
        .await?;
    assert!(!install.status().is_success());
    assert!(install.text().await?.contains("插件已从市场删除"));
    Ok(())
}

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn removal_survives_republication_and_preserves_installations() -> Result<()> {
    let url = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = PgPool::connect(&url).await?;
    let schema = format!("market_removal_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let options = PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new().connect_with(options).await?;
    let result = exercise(&pool).await;
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result
}

async fn exercise(pool: &PgPool) -> Result<()> {
    let store = super::super::store::PluginStore::new(pool.clone());
    store.migrate().await?;
    sqlx::raw_sql(r#"
        INSERT INTO marketplace_entries(source,git,rev,title,summary,license,tags,capabilities)
        VALUES ('aio://published','https://example.com/remove.git','old','Remove','','MIT','[]','{}'),
        ('aio://published','https://example.com/keep.git','keep','Keep','','MIT','[]','{}');
        INSERT INTO plugin_sources(id,git) VALUES ('installed','https://example.com/remove.git');
        INSERT INTO plugin_revisions(id,source_id,revision,runtime,manifest,pages)
        VALUES ('old-revision','installed','old','page-definition','{}','[]');
        INSERT INTO tenant_plugin_bindings(tenant_id,source_id,revision_id)
        VALUES ('tenant-a','installed','old-revision');
        CREATE TABLE business_data(value TEXT);
        INSERT INTO business_data VALUES ('keep');
    "#).execute(pool).await?;
    let git = "https://example.com/remove.git";
    assert!(!is_removed(pool, git).await?);
    mark_removed(pool, git).await?;
    mark_removed(pool, git).await?;
    assert!(is_removed(pool, git).await?);
    assert!(!is_removed(pool, "https://example.com/keep.git").await?);

    let mut entries = store.marketplace_entries("aio://published").await?;
    let mut republished = entries
        .iter()
        .find(|entry| entry.git == git)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("缺少测试条目"))?;
    republished.rev = "new".into();
    let mut transaction = pool.begin().await?;
    super::super::marketplace_store::upsert_published_marketplace_entry(
        &mut transaction,
        &republished,
    )
    .await?;
    transaction.commit().await?;
    store.migrate().await?;
    entries.extend(store.marketplace_entries("aio://published").await?);
    // 模拟已安装目录与组件目录重复回填相同来源。
    republished.installed = true;
    entries.push(republished);
    retain_listed(pool, &mut entries).await?;
    assert!(!entries.is_empty());
    assert!(
        entries
            .iter()
            .all(|entry| entry.git == "https://example.com/keep.git")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tenant_plugin_bindings WHERE revision_id='old-revision'"
        )
        .fetch_one(pool)
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM business_data")
            .fetch_one(pool)
            .await?,
        "keep"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM marketplace_plugin_removals")
            .fetch_one(pool)
            .await?,
        1
    );
    Ok(())
}
