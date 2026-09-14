use anyhow::{Context, Result, ensure};
use az_plugin_development::DevHostSession;
use sqlx::postgres::PgPoolOptions;

/// 在任何运行时迁移之前确认数据库属于当前沙箱，防止误用产品数据库。
pub async fn claim_database(session: &DevHostSession) -> Result<()> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&session.database_url)
        .await
        .context("连接独立开发数据库失败")?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(1729411201)")
        .execute(&mut *tx)
        .await?;
    let owned: bool =
        sqlx::query_scalar("SELECT to_regclass('public.aio_development_owner') IS NOT NULL")
            .fetch_one(&mut *tx)
            .await?;
    let project = session.root.canonicalize()?.to_string_lossy().into_owned();
    if owned {
        let owner: String = sqlx::query_scalar(
            "SELECT workspace FROM public.aio_development_owner WHERE singleton",
        )
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            owner == project,
            "数据库属于另一个开发沙箱，请提供当前项目的独立数据库"
        );
    } else {
        let populated: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND c.relkind IN ('r','p','v','m','S','f'))")
            .fetch_one(&mut *tx).await?;
        ensure!(
            !populated,
            "拒绝迁移已有业务数据的数据库；开发沙箱首次运行必须使用空的独立数据库"
        );
        sqlx::query("CREATE TABLE public.aio_development_owner (singleton BOOLEAN PRIMARY KEY CHECK(singleton), workspace TEXT NOT NULL)").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO public.aio_development_owner VALUES (true, $1)")
            .bind(project)
            .execute(&mut *tx)
            .await?;
    }
    // 插件数据库角色只能访问专属 schema，开发库同样执行正式隔离前提。
    sqlx::query("REVOKE ALL ON SCHEMA public FROM PUBLIC")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    pool.close().await;
    Ok(())
}
