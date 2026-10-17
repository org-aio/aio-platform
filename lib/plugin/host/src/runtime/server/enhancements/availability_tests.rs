//! 缺失语言暂停整包，摘要损坏、数据库失败和可用执行器的编译错误仍必须暴露。

use az_plugin_enhancement::{RhaiExecutor, ScriptRuntime, TypeScriptExecutor};

use super::*;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn language_availability_preserves_revision_without_hiding_invalid_storage() -> Result<()> {
    let pool = PgPool::connect(&database()?).await?;
    let tenant = uuid::Uuid::new_v4().to_string();
    let runtime = Enhancements::open(
        pool.clone(),
        az_plugin_enhancement::default_runtime()?,
        None,
    )
    .await?;
    let mut mixed = package(Language::Rhai, "fn invoke(input, context) { input * 2 }");
    let mut other = package(
        Language::TypeScript,
        "export function invoke(input) { return input * 2; }",
    )
    .enhancements
    .remove(0);
    other.path = "/v1/math/base".into();
    mixed.enhancements.push(other);
    let revision = runtime
        .propose(&tenant, "author", mixed.clone(), async { Ok(()) })
        .await?;
    runtime
        .activate(
            &tenant,
            "author",
            super::super::Activation {
                revision: Some(revision.clone()),
                expected_generation: 0,
            },
            false,
            async { Ok(()) },
        )
        .await?
        .context("激活失败")?;

    for language in [Language::Rhai, Language::TypeScript] {
        let mut services = dill::Catalog::builder();
        services.add::<ScriptRuntime>();
        match language {
            Language::Rhai => {
                services.add::<RhaiExecutor>();
            }
            Language::TypeScript => {
                services.add::<TypeScriptExecutor>();
            }
        }
        let partial = Enhancements::open(
            pool.clone(),
            services.build().get_one::<ScriptRuntime>()?,
            None,
        )
        .await?;
        let (active, contracts) = partial.catalog(&tenant, |_| true, |_| true).await?;
        assert_eq!(active.as_deref(), Some(revision.as_str()));
        assert!(contracts.is_empty());
        assert_eq!(partial.head(&tenant).await?.generation, 1);
        assert!(
            partial
                .invoke(&scope(&tenant), request(), None)
                .await
                .is_err()
        );
        assert!(
            partial
                .propose(&tenant, "author", mixed.clone(), async { Ok(()) })
                .await
                .is_err()
        );

        // 即使缺少另一种语言，也必须先核验持久内容摘要。
        let mut damaged = mixed.clone();
        damaged.enhancements[0].source.push(' ');
        sqlx::query("UPDATE runtime_enhancement_revisions SET package=$3 WHERE tenant_id=$1 AND revision=$2")
            .bind(&tenant).bind(&revision).bind(serde_json::to_value(&damaged)?).execute(&pool).await?;
        let error = partial
            .catalog(&tenant, |_| true, |_| true)
            .await
            .err()
            .context("摘要损坏未被拒绝")?;
        assert!(error.to_string().contains("摘要不匹配"), "{error:#}");
        sqlx::query("UPDATE runtime_enhancement_revisions SET package=$3 WHERE tenant_id=$1 AND revision=$2")
            .bind(&tenant).bind(&revision).bind(serde_json::to_value(&mixed)?).execute(&pool).await?;
    }

    let restored = Enhancements::open(
        pool.clone(),
        az_plugin_enhancement::default_runtime()?,
        None,
    )
    .await?;
    let (active, contracts) = restored.catalog(&tenant, |_| true, |_| true).await?;
    assert_eq!(active.as_deref(), Some(revision.as_str()));
    assert_eq!(contracts.len(), 2);
    assert_eq!(
        restored
            .invoke(&scope(&tenant), request(), None)
            .await?
            .output,
        json!(14)
    );

    // 直接构造摘要一致但无法编译的测试记录，发现不能将所有编译错误当成暂停。
    let invalid_tenant = uuid::Uuid::new_v4().to_string();
    mixed.enhancements[0].source = "fn invoke(".into();
    let invalid_revision = mixed.revision()?;
    sqlx::query("INSERT INTO runtime_enhancement_revisions (tenant_id,revision,package,author) VALUES ($1,$2,$3,'test')")
        .bind(&invalid_tenant).bind(&invalid_revision).bind(serde_json::to_value(&mixed)?).execute(&pool).await?;
    sqlx::query(
        "INSERT INTO runtime_enhancement_heads (tenant_id,revision,generation) VALUES ($1,$2,1)",
    )
    .bind(&invalid_tenant)
    .bind(&invalid_revision)
    .execute(&pool)
    .await?;
    assert!(
        restored
            .catalog(&invalid_tenant, |_| true, |_| true)
            .await
            .is_err()
    );
    pool.close().await;
    assert!(
        restored
            .catalog(&invalid_tenant, |_| true, |_| true)
            .await
            .is_err()
    );
    Ok(())
}
