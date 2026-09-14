use super::super::repository::RepositoryInstaller;
use super::*;
use sqlx::{Connection as _, Executor as _};

fn package(version: &str, artifact: &[u8]) -> Result<PluginPackage> {
    PluginPackage::new(
        "https://github.com/example/offline.git".to_owned(), version.to_owned(), None,
        "[plugin.runtime]\nkind = 'page-definition'\nartifact = 'pages.json'\n[plugin.marketplace]\ntitle = 'Offline'\nsummary = 'Offline binary package'\nlicense = 'MIT'\ntags = ['test']\n[[plugin.subplugins]]\nid = 'offline'\npages = ['offline']\n".to_owned(),
        artifact,
        Default::default(),
    )
}

const PAGES: &[u8] = br#"[{"id":"offline","label":"Offline","scene":{"id":"community","label":"Community"},"body":{"kind":"text","title":"Offline","content":"Binary package"}}]"#;

#[tokio::test]
#[ignore = "需要 AIO_TEST_DATABASE_URL 指向独立测试 PostgreSQL"]
async fn stores_immutable_binary_packages_and_restores_deleted_cache() -> Result<()> {
    let url = std::env::var("AIO_TEST_DATABASE_URL").context("缺少测试 PostgreSQL URL")?;
    let schema = format!("package_test_{}", uuid::Uuid::new_v4().simple());
    let mut connection = sqlx::PgConnection::connect(&url).await?;
    connection
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await?;
    let options_schema = schema.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            let schema = options_schema.clone();
            Box::pin(async move {
                connection
                    .execute(format!("SET search_path TO {schema}").as_str())
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await?;
    let result = async {
        let store = PluginStore::new(pool.clone());
        store.migrate().await?;
        let first = package("1.0.0", PAGES)?;
        store.save_package(&first).await?;
        store.save_package(&first).await?;
        assert!(store.downloadable_package(&first.rev).await?.is_none());
        let conflict = package("1.0.0", format!("{} ", std::str::from_utf8(PAGES)?).as_bytes())?;
        assert!(store.save_package(&conflict).await.is_err());
        let archive = store
            .package_archive(&first.rev)
            .await?
            .context("数据库缺少上传包")?;
        assert_eq!(PluginPackage::decode(&archive)?, first);
        let cache = tempfile::tempdir()?;
        let repository = RepositoryInstaller::new(cache.path().join("runtime"));
        let discovered = repository.publish(&first).await?;
        store
            .activate("test-tenant", discovered, None, None)
            .await?;
        assert_eq!(
            store.downloadable_package(&first.rev).await?,
            Some(archive.clone())
        );
        tokio::fs::remove_dir_all(cache.path().join("runtime")).await?;
        let restored = repository
            .publish(&PluginPackage::decode(&archive)?)
            .await?;
        assert_eq!(restored.revision, first.rev);
        assert!(repository.artifact(&first.rev, "pages.json")?.is_file());
        let second = package("1.0.1", PAGES)?;
        store.save_package(&second).await?;
        assert_ne!(first.rev, second.rev);
        assert!(store.package_archive(&first.rev).await?.is_some());
        assert!(store.package_archive(&second.rev).await?.is_some());
        let source_id = super::super::repository::published_source_id(&first.git);
        store.queue_publish_job("test-tenant", &source_id, &first.git, &first.rev, crate::runtime::PluginRuntime::PageDefinition, 1).await?;
        store.queue_publish_job("test-tenant", &source_id, &second.git, &second.rev, crate::runtime::PluginRuntime::PageDefinition, 1).await?;
        let publication = repository.published_marketplace_entry(&first.git, &first.rev)?;
        let stale = repository.validate_published(&first.git, &first.rev).await?;
        assert!(store.activate("test-tenant", stale, None, Some(&publication)).await
            .expect_err("较早发布不得覆盖后续请求").to_string().contains("后续请求"));
        store.queue_publish_job("test-tenant", &source_id, &first.git, &first.rev, crate::runtime::PluginRuntime::PageDefinition, 1).await?;
        store.activate("test-tenant", repository.validate_published(&first.git, &first.rev).await?, None, Some(&publication)).await?;
        let invalid = package("1.0.2", b"[]")?;
        store.save_package(&invalid).await?;
        assert!(repository.publish(&invalid).await.is_err());
        assert!(store.downloadable_package(&invalid.rev).await?.is_none());
        let active: String = sqlx::query_scalar("SELECT r.revision FROM tenant_plugin_bindings b JOIN plugin_revisions r ON r.id = b.revision_id WHERE b.tenant_id = 'test-tenant'")
            .fetch_one(&pool).await?;
        assert_eq!(active, first.rev);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pool.close().await;
    connection
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await?;
    result
}
