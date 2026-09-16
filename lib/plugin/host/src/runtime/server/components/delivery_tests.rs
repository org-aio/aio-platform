use super::*;
use crate::runtime::server::{RuntimeState, delivery};
use az_plugin_delivery::{BuildJob, BuildReport, Documentation};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL、交付测试令牌和 WIT 测试组件"]
async fn delivery_publishes_bundles_and_respects_tenant_choices() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.path() == "/component_market_test",
        "只接受本机独立测试库"
    );
    let token = std::env::var("AIO_DELIVERY_TOKEN")?;
    let root = tempfile::tempdir()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let identity = Arc::new(super::tests::TestIdentity);
    let mut state =
        RuntimeState::isolated_admin_test(identity.clone(), &database, &base, root.path()).await?;
    let components = Components::open(
        state.store.pool.clone(),
        &database,
        &root.path().join("key.json"),
        root.path().join("objects"),
        identity,
    )
    .await?;
    state.components = Some(components.clone());
    let router = crate::runtime::server::routes::router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = Client::new();
    assert_eq!(
        client
            .post(format!("{base}/api/internal/delivery/claim"))
            .send()
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let git = format!(
        "https://github.com/example/native-delivery-{}.git",
        Uuid::new_v4()
    );
    let package_root = root.path().join("package");
    let create = |job: &BuildJob| -> Result<Bundle> {
        super::tests::package(&package_root, &git, None, &job.version)?;
        Bundle::from_directory(
            &package_root,
            "aio-plugin.toml",
            git.clone(),
            job.source_revision.clone(),
            job.version.clone(),
        )
    };
    let first_job = target(&state, &git, 1).await?;
    let first = create(&first_job)?;
    upload(&client, &base, &token, &first_job, &first).await?;
    delivery::components::tick(&state).await?;
    let (source, published) = components.published(&git, None).await?.context("未发布")?;
    assert_eq!(published.digest, first.digest);
    let tenants: Vec<_> = (0..4).map(|_| Uuid::new_v4().to_string()).collect();
    for tenant in &tenants {
        components.install(tenant, &git, None).await?;
    }
    components.change(&tenants[1], source, "disable").await?;
    components.change(&tenants[2], source, "uninstall").await?;
    let next_job = target(&state, &git, 2).await?;
    let second = create(&next_job)?;
    upload(&client, &base, &token, &next_job, &second).await?;
    delivery::components::tick(&state).await?;
    assert_installed(
        &components,
        &tenants[0],
        source,
        Some((&second.digest, true)),
    )
    .await?;
    assert_installed(
        &components,
        &tenants[1],
        source,
        Some((&first.digest, false)),
    )
    .await?;
    assert_installed(&components, &tenants[2], source, None).await?;
    components.change(&tenants[0], source, "rollback").await?;
    components.rollout().await?;
    assert!(
        !components
            .upgrade(&tenants[0], source, &second.digest)
            .await?
    );
    assert_installed(
        &components,
        &tenants[0],
        source,
        Some((&first.digest, true)),
    )
    .await?;
    let stale_job = target(&state, &git, 3).await?;
    let stale = create(&stale_job)?;
    upload(&client, &base, &token, &stale_job, &stale).await?;
    let current_job = target(&state, &git, 4).await?;
    delivery::components::tick(&state).await?;
    assert_eq!(
        components.published(&git, None).await?.unwrap().1.digest,
        second.digest
    );
    assert!(
        components
            .publish_delivery(stale, "stale", stale_job.id)
            .await
            .is_err()
    );
    let third = create(&current_job)?;
    let mismatch = client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/package",
            current_job.id
        ))
        .bearer_auth(&token)
        .header("x-aio-build-lease", &current_job.lease)
        .header("content-type", "application/vnd.aio.component+gzip")
        .body(first.encode()?)
        .send()
        .await?;
    assert!(!mismatch.status().is_success());
    upload(&client, &base, &token, &current_job, &third).await?;
    delivery::components::tick(&state).await?;
    assert_installed(
        &components,
        &tenants[0],
        source,
        Some((&third.digest, true)),
    )
    .await?;
    assert!(
        !components
            .upgrade(&tenants[1], source, &third.digest)
            .await?
    );
    assert!(
        !components
            .upgrade(&tenants[2], source, &third.digest)
            .await?
    );
    assert!(
        !components
            .upgrade(&tenants[3], source, &second.digest)
            .await?
    );
    let failed_job = target(&state, &git, 5).await?;
    create(&failed_job)?;
    std::fs::copy(
        std::env::var("AIO_TEST_UNHEALTHY_COMPONENT")?,
        package_root.join("plugin.wasm"),
    )?;
    let broken = Bundle::from_directory(
        &package_root,
        "aio-plugin.toml",
        git.clone(),
        failed_job.source_revision.clone(),
        failed_job.version.clone(),
    )?;
    upload(&client, &base, &token, &failed_job, &broken).await?;
    delivery::components::tick(&state).await?;
    assert_eq!(
        components.published(&git, None).await?.unwrap().1.digest,
        third.digest
    );
    assert_installed(
        &components,
        &tenants[0],
        source,
        Some((&third.digest, true)),
    )
    .await?;
    let failed: String = sqlx::query_scalar("SELECT state FROM delivery_jobs WHERE id=$1")
        .bind(failed_job.id)
        .fetch_one(&state.store.pool)
        .await?;
    assert_eq!(failed, "failed");
    let generation: Uuid = sqlx::query_scalar(
        "SELECT generation FROM component_installations WHERE source_id=$1 AND tenant_id=$2",
    )
    .bind(source)
    .bind(&tenants[0])
    .fetch_one(&components.pool)
    .await?;
    components.rollout().await?;
    let unchanged: Uuid = sqlx::query_scalar(
        "SELECT generation FROM component_installations WHERE source_id=$1 AND tenant_id=$2",
    )
    .bind(source)
    .bind(&tenants[0])
    .fetch_one(&components.pool)
    .await?;
    assert_eq!(generation, unchanged);
    for tenant in [&tenants[0], &tenants[1], &tenants[3]] {
        components.change(tenant, source, "uninstall").await?;
    }
    server.abort();
    Ok(())
}

async fn target(state: &RuntimeState, git: &str, sequence: u32) -> Result<BuildJob> {
    let sha = format!("{sequence:040x}");
    let lease = Uuid::new_v4().to_string();
    let recipe = json!({"environment":"fullstack","command":["sh","scripts/build.sh"]});
    let mut tx = state.store.pool.begin().await?;
    sqlx::query("INSERT INTO delivery_sources(git,branch,desired_sha) VALUES($1,'main',$2) ON CONFLICT(git) DO UPDATE SET desired_sha=$2").bind(git).bind(&sha).execute(&mut *tx).await?;
    sqlx::query("UPDATE delivery_jobs SET state='superseded' WHERE git=$1 AND state IN ('building','uploaded','publishing')").bind(git).execute(&mut *tx).await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO delivery_jobs(git,source_revision,recipe,state,lease,lease_until) VALUES($1,$2,$3,'building',$4,now()+interval '5 minutes') RETURNING id")
        .bind(git).bind(&sha).bind(&recipe).bind(&lease).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(BuildJob {
        id,
        lease,
        git: git.into(),
        source_revision: sha.clone(),
        version: format!("0.0.0-dev.{id}+{sha}"),
        recipe: serde_json::from_value(recipe)?,
    })
}

async fn upload(
    client: &Client,
    base: &str,
    token: &str,
    job: &BuildJob,
    bundle: &Bundle,
) -> Result<()> {
    client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/package",
            job.id
        ))
        .bearer_auth(token)
        .header("x-aio-build-lease", &job.lease)
        .header("content-type", "application/vnd.aio.component+gzip")
        .body(bundle.encode()?)
        .send()
        .await?
        .error_for_status()?;
    client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/complete",
            job.id
        ))
        .bearer_auth(token)
        .json(&BuildReport {
            lease: job.lease.clone(),
            error: None,
            documentation: Documentation {
                readme: "# Native delivery".into(),
                ..Default::default()
            },
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn assert_installed(
    components: &Components,
    tenant: &str,
    source: Uuid,
    expected: Option<(&str, bool)>,
) -> Result<()> {
    let value: Option<(String, bool)> = sqlx::query_as(
        "SELECT digest,enabled FROM component_installations WHERE tenant_id=$1 AND source_id=$2",
    )
    .bind(tenant)
    .bind(source)
    .fetch_optional(&components.pool)
    .await?;
    assert_eq!(value.as_ref().map(|(d, e)| (d.as_str(), *e)), expected);
    Ok(())
}
