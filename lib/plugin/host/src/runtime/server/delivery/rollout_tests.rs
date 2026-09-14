use anyhow::{Context as _, Result, ensure};
use az_plugin_delivery::{BuildJob, BuildReport, Documentation};
use az_plugin_package::{PACKAGE_CONTENT_TYPE, PluginPackage};
use reqwest::{Client, StatusCode};
use serde_json::json;

use super::super::{RuntimeState, installation};
use crate::runtime::InstallPluginRequest;

pub(in crate::runtime::server) async fn exercise_rollouts(
    state: &RuntimeState,
    base: &str,
    manifest: &str,
    pages: &str,
) -> Result<()> {
    let token = std::env::var("AIO_DELIVERY_TOKEN").context("交付 HTTP 测试需要测试专用令牌")?;
    let client = Client::new();
    let git = format!(
        "https://github.com/example/delivery-{}.git",
        uuid::Uuid::new_v4().simple()
    );
    let first_job = target(state, &client, base, &token, &git, 1).await?;
    client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/defer",
            first_job.id
        ))
        .bearer_auth(&token)
        .json(&json!({"lease":first_job.lease,"error":"下载暂时不可用"}))
        .send()
        .await?
        .error_for_status()?;
    let pending = client
        .post(format!("{base}/api/internal/delivery/claim"))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .json::<Option<BuildJob>>()
        .await?;
    assert!(pending.is_none(), "退避中的任务不能立即重复领取");
    let independent_git = format!("{git}-independent");
    let independent = target(state, &client, base, &token, &independent_git, 1).await?;
    assert_eq!(independent.git, independent_git, "网络失败不得阻塞其他插件");
    client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/complete",
            independent.id
        ))
        .bearer_auth(&token)
        .json(&BuildReport {
            lease: independent.lease,
            error: Some("测试结束".into()),
            documentation: Documentation::default(),
        })
        .send()
        .await?
        .error_for_status()?;
    sqlx::query("UPDATE delivery_jobs SET next_attempt_at=now() WHERE id=$1")
        .bind(first_job.id)
        .execute(&state.store.pool)
        .await?;
    let resumed = client
        .post(format!("{base}/api/internal/delivery/claim"))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .json::<Option<BuildJob>>()
        .await?
        .context("退避结束后必须恢复")?;
    assert_eq!(resumed.id, first_job.id);
    assert_ne!(resumed.lease, first_job.lease);
    let first_job = resumed;
    let first = publish(state, &client, base, &token, &first_job, manifest, pages).await?;
    let source = state
        .store
        .source_id(&git)
        .await?
        .context("首次发布必须自动安装")?;
    assert_binding(state, "default", &source, Some((&first, true))).await?;
    let tenants: Vec<_> = (0..4)
        .map(|i| format!("delivery-{}-{i}", uuid::Uuid::new_v4().simple()))
        .collect();
    for tenant in &tenants {
        installation::install(
            state,
            tenant,
            &InstallPluginRequest {
                git: git.clone(),
                rev: None,
            },
        )
        .await?;
    }
    state
        .store
        .set_enabled("default", &source, false, None)
        .await?;
    state
        .store
        .set_enabled(&tenants[1], &source, false, None)
        .await?;
    state.store.uninstall(&tenants[2], &source).await?;

    let failed = target(state, &client, base, &token, &git, 2).await?;
    client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/complete",
            failed.id
        ))
        .bearer_auth(&token)
        .json(&BuildReport {
            lease: failed.lease,
            error: Some("编译错误测试".into()),
            documentation: Documentation::default(),
        })
        .send()
        .await?
        .error_for_status()?;
    assert_binding(state, &tenants[0], &source, Some((&first, true))).await?;
    let status: String = sqlx::query_scalar("SELECT state FROM delivery_jobs WHERE id=$1")
        .bind(failed.id)
        .fetch_one(&state.store.pool)
        .await?;
    assert_eq!(status, "failed");

    let second_job = target(state, &client, base, &token, &git, 3).await?;
    let second = publish(state, &client, base, &token, &second_job, manifest, pages).await?;
    sqlx::query("INSERT INTO delivery_rollouts(tenant_id,source_id,revision,state,error) VALUES($1,$2,$3,'failed','租户暂时不可用')")
        .bind(&tenants[0]).bind(&source).bind(&second).execute(&state.store.pool).await?;
    super::rollout::tick(state).await?;
    assert_binding(state, &tenants[0], &source, Some((&first, true))).await?;
    assert_binding(state, &tenants[3], &source, Some((&second, true))).await?;
    sqlx::query("UPDATE delivery_rollouts SET updated_at=now()-interval '2 minutes' WHERE tenant_id=$1 AND source_id=$2")
        .bind(&tenants[0]).bind(&source).execute(&state.store.pool).await?;
    super::rollout::tick(state).await?;
    assert_binding(state, "default", &source, Some((&first, false))).await?;
    assert_binding(state, &tenants[0], &source, Some((&second, true))).await?;
    assert_binding(state, &tenants[1], &source, Some((&first, false))).await?;
    assert_binding(state, &tenants[2], &source, None).await?;
    let previous = state.store.rollback_target(&tenants[0], &source).await?;
    assert_eq!(previous.revision, first);
    state
        .store
        .rollback_to(&tenants[0], &source, &previous, None)
        .await?;
    super::rollout::tick(state).await?;
    assert_binding(state, &tenants[0], &source, Some((&first, true))).await?;

    let obsolete = target(state, &client, base, &token, &git, 4).await?;
    let newest = target(state, &client, base, &token, &git, 5).await?;
    let heartbeat = client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/heartbeat",
            obsolete.id
        ))
        .bearer_auth(&token)
        .json(&json!({"lease":obsolete.lease}))
        .send()
        .await?;
    assert_eq!(heartbeat.status(), StatusCode::CONFLICT);
    let third = publish(state, &client, base, &token, &newest, manifest, pages).await?;
    super::rollout::tick(state).await?;
    assert_binding(state, &tenants[0], &source, Some((&third, true))).await?;
    assert_binding(state, &tenants[1], &source, Some((&first, false))).await?;
    assert_binding(state, &tenants[2], &source, None).await?;
    assert_binding(state, "default", &source, Some((&first, false))).await?;
    installation::install(
        state,
        "default",
        &InstallPluginRequest {
            git: git.clone(),
            rev: None,
        },
    )
    .await?;
    assert_binding(state, "default", &source, Some((&third, true))).await?;
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM plugin_lifecycle_events WHERE source_id=$1 AND lifecycle='activate'",
    )
    .bind(&source)
    .fetch_one(&state.store.pool)
    .await?;
    super::rollout::tick(state).await?;
    let after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM plugin_lifecycle_events WHERE source_id=$1 AND lifecycle='activate'",
    )
    .bind(&source)
    .fetch_one(&state.store.pool)
    .await?;
    assert_eq!(before, after, "无变化轮询不得再次激活");
    println!(
        "交付 HTTP 验证通过: 首次安装、失败保留、多租户升级、停用/卸载、回滚与新提交、过期构建拒绝、无变化轮询"
    );
    Ok(())
}

async fn target(
    state: &RuntimeState,
    client: &Client,
    base: &str,
    token: &str,
    git: &str,
    sequence: u32,
) -> Result<BuildJob> {
    let sha = format!("{sequence:040x}");
    let mut tx = state.store.pool.begin().await?;
    sqlx::query("INSERT INTO delivery_sources(git,branch,desired_sha) VALUES($1,'main',$2) ON CONFLICT(git) DO UPDATE SET desired_sha=$2")
        .bind(git).bind(&sha).execute(&mut *tx).await?;
    sqlx::query("UPDATE delivery_jobs SET state='superseded',lease_until=NULL WHERE git=$1 AND state IN ('queued','building','uploaded','publishing')")
        .bind(git).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO delivery_jobs(git,source_revision,recipe) VALUES($1,$2,$3)")
        .bind(git)
        .bind(&sha)
        .bind(json!({"environment":"typescript","command":["sh","scripts/build.sh"]}))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    client
        .post(format!("{base}/api/internal/delivery/claim"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json::<Option<BuildJob>>()
        .await?
        .context("应领取目标构建")
}

async fn publish(
    state: &RuntimeState,
    client: &Client,
    base: &str,
    token: &str,
    job: &BuildJob,
    manifest: &str,
    pages: &str,
) -> Result<String> {
    let package = PluginPackage::new(
        job.git.clone(),
        job.version.clone(),
        Some(job.source_revision.clone()),
        manifest.into(),
        pages.as_bytes(),
        Default::default(),
    )?;
    client
        .post(format!(
            "{base}/api/internal/delivery/jobs/{}/package",
            job.id
        ))
        .bearer_auth(token)
        .header("x-aio-build-lease", &job.lease)
        .header("content-type", PACKAGE_CONTENT_TYPE)
        .body(package.encode()?)
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
                readme: format!("# {}", job.source_revision),
                images: Default::default(),
            },
        })
        .send()
        .await?
        .error_for_status()?;
    for _ in 0..100 {
        if state
            .store
            .published_marketplace_entry(&job.git, None)
            .await?
            .is_some_and(|entry| entry.rev == package.rev)
        {
            return Ok(package.rev);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    anyhow::bail!("交付发布未完成: {}", job.id)
}

async fn assert_binding(
    state: &RuntimeState,
    tenant: &str,
    source: &str,
    expected: Option<(&str, bool)>,
) -> Result<()> {
    let binding: Option<(String,bool)> = sqlx::query_as("SELECT r.revision,b.enabled FROM tenant_plugin_bindings b JOIN plugin_revisions r ON r.id=b.revision_id WHERE b.tenant_id=$1 AND b.source_id=$2")
        .bind(tenant).bind(source).fetch_optional(&state.store.pool).await?;
    ensure!(
        binding
            .as_ref()
            .map(|(revision, enabled)| (revision.as_str(), *enabled))
            == expected,
        "租户 {tenant} 绑定不符合预期: {binding:?} / {expected:?}"
    );
    Ok(())
}
