#![cfg(all(
    feature = "rhai",
    feature = "typescript",
    feature = "mcp",
    feature = "grpc"
))]

#[path = "enhancement_catalog/file.rs"]
mod file;
#[path = "enhancement_catalog/http.rs"]
mod http;
#[path = "support/identity.rs"]
mod identity_support;
#[path = "enhancement_catalog/transactions.rs"]
mod transactions;

use anyhow::Result;
use az_plugin_enhancement::{Enhancement, Example, Language, Package};
use az_plugin_host::runtime::server::enhancement_catalog::*;
use serde_json::json;

const PATH: &str = "/v1/math/double";

fn package(language: Language) -> Package {
    Package {
        schema_version: 1,
        enhancements: vec![Enhancement {
            path: PATH.into(),
            language,
            source: match language {
                Language::Rhai => "fn invoke(input, context) { input * 2 }",
                Language::TypeScript => {
                    "export function invoke(input: number) { return input * 2; }"
                }
            }
            .into(),
            calls: vec![],
            capabilities: vec![],
            input_schema: None,
            output_schema: None,
            examples: vec![Example {
                input: json!(3),
                output: json!(6),
                capabilities: vec![],
            }],
        }],
    }
}

fn authorize() -> CommitAuthorization<'static> {
    Box::pin(async { Ok(()) })
}

#[tokio::test]
async fn file_catalog_transactions_preserve_authorization_and_history() -> Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = file::FileCatalog::open(&root.path().join("catalog.json"));
    transactions::exercise(catalog.clone(), catalog).await
}

#[tokio::test]
#[cfg(feature = "postgres")]
#[ignore = "需要独立本机 PostgreSQL"]
async fn postgres_catalog_transactions_work_across_connections() -> Result<()> {
    let url = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    anyhow::ensure!(
        url.contains("127.0.0.1") && url.contains("_test"),
        "只接受本机独立测试库"
    );
    let left = sqlx::PgPool::connect(&url).await?;
    let right = sqlx::PgPool::connect(&url).await?;
    let first = std::sync::Arc::new(PostgresEnhancementCatalog::open(left.clone()).await?);
    let second = std::sync::Arc::new(PostgresEnhancementCatalog::open(right.clone()).await?);
    let result = transactions::exercise(first, second).await;
    left.close().await;
    right.close().await;
    result
}

#[tokio::test]
#[cfg(feature = "postgres")]
#[ignore = "需要独立本机 PostgreSQL"]
async fn postgres_commit_failure_rolls_back_candidates_heads_and_events() -> Result<()> {
    use anyhow::Context;
    let url = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    anyhow::ensure!(
        url.contains("127.0.0.1") && url.contains("_test"),
        "只接受本机独立测试库"
    );
    let pool = sqlx::PgPool::connect(&url).await?;
    let catalog = PostgresEnhancementCatalog::open(pool.clone()).await?;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let function = format!("reject_enhancement_{suffix}");
    let candidate_trigger = format!("candidate_{suffix}");
    let activation_trigger = format!("activation_{suffix}");
    sqlx::raw_sql(&format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected commit failure'; END $$;
        CREATE CONSTRAINT TRIGGER {candidate_trigger} AFTER INSERT ON runtime_enhancement_revisions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.author = '{suffix}') EXECUTE FUNCTION {function}();
        CREATE CONSTRAINT TRIGGER {activation_trigger} AFTER INSERT ON runtime_enhancement_events DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.actor = '{suffix}') EXECUTE FUNCTION {function}();"))
        .execute(&pool).await?;
    let outcome = async {
        let package = package(Language::Rhai);
        let revision = package.revision()?;
        assert!(
            catalog
                .propose(&suffix, &suffix, &package, authorize())
                .await
                .is_err()
        );
        assert!(catalog.revisions(&suffix).await?.is_empty());
        catalog
            .propose(&suffix, "author", &package, authorize())
            .await?;
        let request = Activation {
            revision: Some(revision),
            expected_generation: 0,
        };
        assert!(
            catalog
                .activate(&suffix, &suffix, &request, false, authorize())
                .await
                .is_err()
        );
        assert_eq!(catalog.head(&suffix).await?, Head::default());
        assert!(catalog.history(&suffix).await?.is_empty());
        assert_eq!(
            catalog
                .activate(&suffix, "actor", &request, false, authorize())
                .await?
                .context("故障恢复后激活失败")?
                .generation,
            1
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    sqlx::raw_sql(&format!("DROP TRIGGER {candidate_trigger} ON runtime_enhancement_revisions; DROP TRIGGER {activation_trigger} ON runtime_enhancement_events; DROP FUNCTION {function}();")).execute(&pool).await?;
    pool.close().await;
    outcome
}
