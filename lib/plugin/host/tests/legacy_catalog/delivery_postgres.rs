//! 默认协调器通过真实 HTTP 使用原目录事务，提交失败不能泄漏发布任务。
use super::super::fixture::{database, plugins};
use super::delivery_http::{artifact, build_request, claim, upload};
use super::*;
use az_plugin_delivery::{BuildReport, Documentation};
use az_plugin_host::runtime::server::delivery::DeliveryService;
use sqlx::PgPool;

#[tokio::test]
#[ignore = "需要独立 PostgreSQL 和 AIO_DELIVERY_TOKEN"]
async fn postgres_delivery_keeps_publication_atomic_and_rejects_stale_leases() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let mut selected = plugins(Default::default(), true);
    selected.extend([
        Arc::new(PostgresDocumentationPlugin) as DynPlugin<RuntimeAssembly>,
        Arc::new(PostgresDeliveryPlugin),
        Arc::new(DeliveryPlugin),
        Arc::new(DocumentationPlugin),
    ]);
    let selected = identity_support::with_identity(Arc::new(Identity("default".into())), selected);
    let host = Host::start_selected(root.path(), &database, selected).await?;
    let pool = host.state.service::<PgPool>()?;
    let git = format!(
        "https://github.com/example/pg-delivery-{}.git",
        uuid::Uuid::new_v4()
    );
    sqlx::query("INSERT INTO delivery_sources(git,branch,desired_sha) VALUES($1,'main',$2)")
        .bind(&git)
        .bind("a".repeat(40))
        .execute(pool.as_ref())
        .await?;
    sqlx::query("INSERT INTO delivery_jobs(git,source_revision,recipe) VALUES($1,$2,$3)")
        .bind(&git)
        .bind("a".repeat(40))
        .bind(json!({"environment":"typescript","command":["fixture"]}))
        .execute(pool.as_ref())
        .await?;
    let job = claim(&host).await?;
    let package = artifact(&job)?;
    let upload_trigger = format!("upload_commit_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE FUNCTION {upload_trigger}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id={} AND NEW.state='uploaded' THEN RAISE EXCEPTION 'fixture upload failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER {upload_trigger} BEFORE UPDATE ON delivery_jobs FOR EACH ROW EXECUTE FUNCTION {upload_trigger}();",job.id)).execute(pool.as_ref()).await?;
    let rejected = upload(&host, &job, &package).await?;
    assert!(!rejected.status().is_success());
    assert!(rejected.text().await?.contains("fixture upload failure"));
    let archives: i64 =
        sqlx::query_scalar("SELECT count(*) FROM plugin_packages WHERE revision=$1")
            .bind(&package.rev)
            .fetch_one(pool.as_ref())
            .await?;
    assert_eq!(archives, 0, "失败上传不能占用不可变版本号");
    sqlx::raw_sql(&format!(
        "DROP TRIGGER {upload_trigger} ON delivery_jobs; DROP FUNCTION {upload_trigger}();"
    ))
    .execute(pool.as_ref())
    .await?;
    upload(&host, &job, &package).await?.error_for_status()?;
    let report = BuildReport {
        lease: job.lease.clone(),
        error: None,
        documentation: Documentation {
            readme: "# PostgreSQL".into(),
            images: Default::default(),
        },
    };
    let route = format!("/jobs/{}/complete", job.id);
    // 故障发生在发布任务写入之后，交付状态提交之前；两者必须一起回滚。
    let trigger = format!("delivery_commit_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE FUNCTION {trigger}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id={} AND NEW.state='publishing' THEN RAISE EXCEPTION 'fixture commit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER {trigger} BEFORE UPDATE ON delivery_jobs FOR EACH ROW EXECUTE FUNCTION {trigger}();",job.id)).execute(pool.as_ref()).await?;
    let failed = build_request(&host, &route)?.json(&report).send().await?;
    assert!(!failed.status().is_success());
    assert!(failed.text().await?.contains("fixture commit failure"));
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM plugin_publish_jobs WHERE git=$1")
        .bind(&git)
        .fetch_one(pool.as_ref())
        .await?;
    assert_eq!(jobs, 0);
    let status: String = sqlx::query_scalar("SELECT state FROM delivery_jobs WHERE id=$1")
        .bind(job.id)
        .fetch_one(pool.as_ref())
        .await?;
    assert_eq!(status, "uploaded");
    sqlx::raw_sql(&format!(
        "DROP TRIGGER {trigger} ON delivery_jobs; DROP FUNCTION {trigger}();"
    ))
    .execute(pool.as_ref())
    .await?;
    build_request(&host, &route)?
        .json(&report)
        .send()
        .await?
        .error_for_status()?;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let states: Vec<String> =
                sqlx::query_scalar("SELECT state FROM plugin_publish_jobs WHERE git=$1")
                    .bind(&git)
                    .fetch_all(pool.as_ref())
                    .await?;
            if states.iter().any(|s| s == "active") {
                return anyhow::Ok(());
            }
            anyhow::ensure!(!states.iter().any(|s| s == "failed"), "发布失败");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    let details = get(
        &host,
        &format!("/api/runtime/marketplace/{}/details", package.rev),
    )
    .await?;
    assert_eq!(details["data"]["readme"], "# PostgreSQL");
    assert!(
        !host
            .state
            .service::<dyn DeliveryService>()?
            .heartbeat(job.id, &job.lease)
            .await?
    );
    // 新任务租约超时后不得借心跳复活，也不得用旧上传覆盖新工作者。
    sqlx::query("UPDATE delivery_sources SET desired_sha=$2 WHERE git=$1")
        .bind(&git)
        .bind("b".repeat(40))
        .execute(pool.as_ref())
        .await?;
    sqlx::query("INSERT INTO delivery_jobs(git,source_revision,recipe) VALUES($1,$2,$3)")
        .bind(&git)
        .bind("b".repeat(40))
        .bind(json!({"environment":"typescript","command":["fixture"]}))
        .execute(pool.as_ref())
        .await?;
    let old = claim(&host).await?;
    sqlx::query("UPDATE delivery_jobs SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(old.id)
        .execute(pool.as_ref())
        .await?;
    assert_eq!(
        build_request(&host, &format!("/jobs/{}/heartbeat", old.id))?
            .json(&json!({"lease":old.lease}))
            .send()
            .await?
            .status(),
        409
    );
    let new = claim(&host).await?;
    assert_ne!(old.lease, new.lease);
    assert!(
        !upload(&host, &old, &artifact(&old)?)
            .await?
            .status()
            .is_success()
    );
    upload(&host, &new, &artifact(&new)?)
        .await?
        .error_for_status()?;
    sqlx::query("UPDATE delivery_sources SET desired_sha=$2 WHERE git=$1")
        .bind(&git)
        .bind("c".repeat(40))
        .execute(pool.as_ref())
        .await?;
    assert!(
        !host
            .state
            .service::<dyn DeliveryService>()?
            .heartbeat(new.id, &new.lease)
            .await?
    );
    assert!(
        !build_request(&host, &format!("/jobs/{}/complete", new.id))?
            .json(&BuildReport {
                lease: new.lease,
                error: Some("obsolete".into()),
                documentation: Documentation::default()
            })
            .send()
            .await?
            .status()
            .is_success()
    );
    sqlx::query("DELETE FROM delivery_jobs WHERE git=$1")
        .bind(&git)
        .execute(pool.as_ref())
        .await?;
    sqlx::query("DELETE FROM delivery_sources WHERE git=$1")
        .bind(&git)
        .execute(pool.as_ref())
        .await?;
    let delivery = host.state.service::<dyn DeliveryService>()?;
    host.stop().await?;
    assert!(
        delivery.heartbeat(old.id, &old.lease).await.is_err(),
        "连接池关闭必须返回真实错误"
    );
    Ok(())
}
