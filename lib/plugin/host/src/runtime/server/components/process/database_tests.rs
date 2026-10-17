use std::{path::Path, str::FromStr, sync::Arc};

use anyhow::{Context, Result};
use az_plugin_runtime::{ComponentDatabase, DatabaseEndpoint, DatabaseProvisioner};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::{net::UnixListener, task::JoinSet};

fn relay(path: &Path, endpoint: DatabaseEndpoint, jobs: &mut JoinSet<Result<()>>) -> Result<()> {
    let listener = UnixListener::bind(path)?;
    let endpoint = Arc::new(endpoint);
    jobs.spawn(async move {
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                client = listener.accept() => {
                    let (stream, _) = client?;
                    clients.spawn(super::configuration::forward_database(stream, endpoint.clone()));
                }
                result = clients.join_next(), if !clients.is_empty() => {
                    // 单个客户端关闭连接不能终止整个测试代理；与实际 broker 保持相同生命周期。
                    if let Err(error) = result.context("数据库代理任务缺失")?? {
                        eprintln!("测试数据库连接已关闭: {error:#}");
                    }
                }
            }
        }
    });
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL，验证真实 Sidecar TCP 和 Unix 转发"]
async fn sidecar_database_tunnel_preserves_checked_role_through_both_transports() -> Result<()> {
    let provisioner =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let source = uuid::Uuid::new_v4().to_string();
    let database = provisioner
        .create(
            &source,
            "tenant",
            "CREATE TABLE records(id BIGINT PRIMARY KEY,value TEXT)",
        )
        .await?;
    let connection = database.process_connection()?;
    let mut transaction = database.begin().await?;
    transaction
        .execute("INSERT INTO records VALUES(1,'from-runtime')", vec![])
        .await?;
    transaction.commit().await?;
    let expected = PgConnectOptions::from_str(&connection.direct_url)?
        .get_username()
        .to_owned();
    let upstream = tempfile::tempdir()?;
    let downstream = tempfile::tempdir()?;
    let upstream_socket = upstream.path().join(&connection.socket_name);
    let downstream_socket = downstream.path().join(&connection.socket_name);
    let mut jobs = JoinSet::new();
    relay(&upstream_socket, connection.upstream, &mut jobs)?;
    relay(
        &downstream_socket,
        DatabaseEndpoint::Unix(upstream_socket),
        &mut jobs,
    )?;
    for (transport, directory) in [("TCP", upstream.path()), ("Unix", downstream.path())] {
        let options = PgConnectOptions::from_str(&connection.isolated_url)?.socket(directory);
        let pool: PgPool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .with_context(|| format!("连接 {transport} Sidecar 数据库代理失败"))?;
        let role: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&pool)
            .await?;
        assert_eq!(role, expected);
        let value: String = sqlx::query_scalar("SELECT value FROM records WHERE id=1")
            .fetch_one(&pool)
            .await?;
        assert_eq!(value, "from-runtime");
        assert!(
            sqlx::query("SELECT * FROM aio_plugin_host.component_releases")
                .fetch_all(&pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("CREATE TABLE forbidden(id BIGINT)")
                .execute(&pool)
                .await
                .is_err()
        );
        pool.close().await;
    }
    jobs.shutdown().await;
    provisioner.shutdown().await?;
    Ok(())
}
