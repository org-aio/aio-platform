use anyhow::{Context as _, Result};
use sqlx::PgPool;

#[cfg(test)]
mod tests;

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::raw_sql(include_str!("migration.sql"))
        .execute(&mut *transaction)
        .await
        .context("备份并清理第三方市场源失败")?;
    transaction.commit().await?;
    Ok(())
}
