//! 构建完成后的发布推进属于交付服务；停用自动升级仍可完成新包发布。
use super::super::RuntimeState;
use anyhow::Result;
use tokio_util::sync::CancellationToken;

pub(super) async fn run(state: RuntimeState, cancellation: CancellationToken) -> Result<()> {
    while !cancellation.is_cancelled() {
        if let Err(error) = tick(&state).await {
            eprintln!("交付发布推进失败: {error:#}");
        }
        tokio::select! {
            _ = cancellation.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
        }
    }
    Ok(())
}
pub(super) async fn tick(state: &RuntimeState) -> Result<()> {
    super::components::publish_pending(state).await?;
    sqlx::query("UPDATE delivery_jobs j SET state=CASE p.state WHEN 'active' THEN 'active' ELSE 'failed' END,error=CASE WHEN p.state='failed' THEN p.detail ELSE NULL END,updated_at=now() FROM plugin_publish_jobs p WHERE j.package_revision=p.revision AND j.state='publishing' AND p.state IN ('active','failed')").execute(state.service::<sqlx::PgPool>()?.as_ref()).await?;
    Ok(())
}
