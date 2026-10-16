use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use az_plugin_contract::grpc::{InvokeRequest, enhancements_client::EnhancementsClient};
use az_plugin_enhancement::Language;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::support::{Fixture, auth, mcp_client};
use crate::runtime::server::{enhancements::Enhancements, plugins::EvolutionPlugin};

#[path = "evolution_fixture.rs"]
mod fixture;
use fixture::*;

async fn invoke(
    fixture: &Fixture,
    mcp: bool,
    actor: &str,
    path: &str,
    input: Value,
) -> Result<Value> {
    if mcp {
        let client = mcp_client(&fixture.base, actor).await?;
        let response = client
            .call_tool(
                CallToolRequestParams::new(path.trim_start_matches('/').replace('/', "."))
                    .with_arguments(serde_json::Map::from_iter([("input".into(), input)])),
            )
            .await?;
        client.cancel().await?;
        ensure!(response.is_error != Some(true), "MCP 演进调用被拒绝");
        return response.structured_content.context("缺少 MCP 结果");
    }
    let mut client = EnhancementsClient::connect(fixture.base.clone()).await?;
    let response = client
        .invoke(auth(
            InvokeRequest {
                path: path.into(),
                input_json: serde_json::to_vec(&input)?,
            },
            actor,
        )?)
        .await?
        .into_inner();
    Ok(
        json!({"revision":response.revision,"output":serde_json::from_slice::<Value>(&response.output_json)?}),
    )
}

async fn control(fixture: &Fixture, operation: &str, value: Value) -> Result<Value> {
    Ok(invoke(
        fixture,
        false,
        "evolution",
        CONTROL,
        json!({"path":capability(operation),"value":value}),
    )
    .await?["output"]
        .clone())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn policies_evolve_observe_rollback_and_replace_themselves_over_both_protocols() -> Result<()>
{
    for language in [Language::Rhai, Language::TypeScript] {
        for mcp in [false, true] {
            let generator = Arc::new(Generator::default());
            let fixture = Fixture::with_plugins(vec![
                Arc::new(EvolutionPlugin),
                Arc::new(GeneratorPlugin(generator.clone())),
            ])
            .await?;
            let outcome = tokio::time::timeout(
                Duration::from_secs(60),
                exercise(&fixture, &generator, language, mcp),
            )
            .await;
            let stopped = fixture.state.shutdown().await;
            fixture.server.abort();
            outcome??;
            stopped?;
        }
    }
    Ok(())
}

async fn exercise(
    fixture: &Fixture,
    generator: &Generator,
    language: Language,
    mcp: bool,
) -> Result<()> {
    let service = fixture.state.service::<Enhancements>()?;
    let tenant = &fixture.identity.tenant;
    let first = fixture.publish(fixture::package(language)?, 0).await?;
    assert_eq!(
        generator.calls.load(Ordering::SeqCst),
        0,
        "样例重放不能调用模型"
    );
    let input = json!({"expected_generation":1,"requirement":"break","target":target(language)});
    // 显式 runtime 授权不能替代管理权限，管理员也不能自动继承 runtime 授权。
    assert!(
        invoke(fixture, mcp, "evolution-reader", BOOTSTRAP, input.clone())
            .await
            .is_err()
    );
    assert!(
        invoke(fixture, mcp, "manager", BOOTSTRAP, input.clone())
            .await
            .is_err()
    );
    assert_eq!(generator.calls.load(Ordering::SeqCst), 0);

    let response = invoke(fixture, mcp, "evolution", BOOTSTRAP, input.clone())
        .await
        .context("生成并激活候选失败")?;
    assert_eq!(response["revision"], first, "发起自更新的调用仍使用旧快照");
    assert_eq!(response["output"]["generation"], 2);
    let second = response["output"]["revision"]
        .as_str()
        .context("缺少新版本")?
        .to_owned();
    assert_eq!(generator.calls.load(Ordering::SeqCst), 1);
    assert!(
        invoke(fixture, mcp, "evolution", BOOTSTRAP, input)
            .await
            .is_err()
    );
    assert_eq!(
        generator.calls.load(Ordering::SeqCst),
        1,
        "旧代数不得重试模型"
    );
    assert!(
        invoke(fixture, mcp, "evolution", MATH, json!(7))
            .await
            .is_err()
    );
    let samples = control(
        fixture,
        "observe",
        json!({"revision":second,"generation":2}),
    )
    .await?;
    let sample = samples
        .as_array()
        .context("缺少观察记录")?
        .iter()
        .find(|entry| entry["path"] == MATH)
        .context("缺少失败计数")?;
    assert_eq!(sample["failed"], 1);
    assert_eq!(sample["succeeded"], 0);
    let mut observation = json!({"revision":second,"expected_generation":2,"path":MATH,"previous_revision":first,"minimum_completed":2,"failure_ratio":1.0});
    let unchanged = invoke(fixture, mcp, "evolution", OBSERVE, observation.clone())
        .await
        .context("观察未达阈值失败")?;
    assert_eq!(unchanged["output"]["rolled_back"], false);
    observation["minimum_completed"] = json!(1);
    let restored = invoke(fixture, mcp, "evolution", OBSERVE, observation)
        .await
        .context("观察触发回滚失败")?;
    assert_eq!(restored["revision"], second);
    assert_eq!(restored["output"]["rolled_back"], true);
    assert_eq!(restored["output"]["head"]["revision"], first);
    assert_eq!(restored["output"]["head"]["generation"], 3);
    assert_eq!(
        invoke(fixture, mcp, "evolution", MATH, json!(7)).await?["output"],
        14
    );
    let prior = control(fixture, "observe", json!({"revision":first,"generation":1})).await?;
    assert!(
        prior
            .as_array()
            .context("缺少首轮观察")?
            .iter()
            .all(|entry| entry["path"] != MATH)
    );
    let restored_samples =
        control(fixture, "observe", json!({"revision":first,"generation":3})).await?;
    assert_eq!(
        restored_samples
            .as_array()
            .context("缺少回滚后观察")?
            .iter()
            .find(|entry| entry["path"] == MATH)
            .context("缺少回滚后调用")?["succeeded"],
        1
    );
    assert!(
        control(fixture, "observe", json!({"revision":first,"generation":2}))
            .await
            .is_err()
    );

    let history = control(fixture, "history", json!({})).await?;
    assert_eq!(history[0]["action"], "rollback");
    assert_eq!(history[0]["actor"], "evolution");
    assert_eq!(history[1]["action"], "activate");
    // 替换策略自身仍须通过旧样例，模型没有改写自身授权或验收条件的接口。
    let package = service.package(tenant, &first).await?;
    let mut policy = serde_json::to_value(
        package
            .enhancements
            .iter()
            .find(|entry| entry.path == BOOTSTRAP)
            .context("缺少自举策略")?,
    )?;
    policy["language"] = json!("typescript");
    policy["source"] = json!("");
    // 序列化可能省略空数组；生成请求的模板显式声明依赖。
    policy["calls"] = json!([]);
    let changed = invoke(
        fixture,
        mcp,
        "evolution",
        BOOTSTRAP,
        json!({"expected_generation":3,"requirement":"self","target":policy}),
    )
    .await?;
    let updated = changed["output"]["revision"]
        .as_str()
        .context("缺少策略版本")?;
    assert_eq!(changed["output"]["generation"], 4);
    assert_eq!(
        service
            .package(tenant, updated)
            .await?
            .enhancements
            .iter()
            .find(|entry| entry.path == BOOTSTRAP)
            .context("新策略丢失")?
            .language,
        Language::TypeScript
    );
    assert!(
        invoke(
            fixture,
            mcp,
            "evolution",
            BOOTSTRAP,
            json!({"expected_generation":4,"requirement":"invalid","target":target(language)})
        )
        .await
        .is_err()
    );
    assert_eq!(
        service.head(tenant).await?.revision.as_deref(),
        Some(updated)
    );
    assert_eq!(
        service.revisions(tenant).await?.len(),
        3,
        "失败候选不能落库"
    );

    let mut foreign = serde_json::to_value(service.package(tenant, updated).await?)?;
    foreign["tenant_id"] = json!("forged");
    assert!(control(fixture, "propose", foreign).await.is_err());
    assert!(
        control(fixture, "package", json!({"revision":"other-tenant"}))
            .await
            .is_err()
    );
    let mut widened = service.package(tenant, updated).await?;
    widened.enhancements[0]
        .capabilities
        .push("/v1/ungranted/write".into());
    assert!(
        control(fixture, "propose", serde_json::to_value(widened)?)
            .await
            .is_err()
    );
    fixture
        .identity
        .management_allowed
        .store(false, Ordering::SeqCst);
    assert!(control(fixture, "head", json!({})).await.is_err());
    fixture
        .identity
        .management_allowed
        .store(true, Ordering::SeqCst);
    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert!(control(fixture, "head", json!({})).await.is_err());
    fixture.identity.revoked.store(false, Ordering::SeqCst);
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn cancellation_and_commit_time_revocation_do_not_publish_candidates() -> Result<()> {
    let generator = Arc::new(Generator::default());
    let fixture = Fixture::with_plugins(vec![
        Arc::new(EvolutionPlugin),
        Arc::new(GeneratorPlugin(generator.clone())),
    ])
    .await?;
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        let first = fixture.publish(fixture::package(Language::TypeScript)?,0).await?;
        let service = fixture.state.service::<Enhancements>()?;
        let tenant = &fixture.identity.tenant;
        let mut client = EnhancementsClient::connect(fixture.base.clone()).await?;
        let mut request = auth(InvokeRequest {path:BOOTSTRAP.into(),input_json:serde_json::to_vec(&json!({"expected_generation":1,"requirement":"wait","target":target(Language::Rhai)}))?},"evolution")?;
        request.set_timeout(Duration::from_millis(350));
        assert!(client.invoke(request).await.is_err());
        tokio::time::timeout(Duration::from_secs(2),async {
            while generator.active.load(Ordering::SeqCst)!=0 { tokio::time::sleep(Duration::from_millis(5)).await; }
        }).await?;
        assert_eq!(generator.calls.load(Ordering::SeqCst),1);
        assert_eq!(service.head(tenant).await?.revision.as_deref(),Some(first.as_str()));
        assert_eq!(service.revisions(tenant).await?.len(),1);

        // 持有活动指针行锁，确保请求已经通过初始鉴权与契约检查后才撤销管理角色。
        let pool = fixture.state.service::<sqlx::PgPool>()?;
        let mut lock = pool.begin().await?;
        sqlx::query("SELECT generation FROM runtime_enhancement_heads WHERE tenant_id=$1 FOR UPDATE").bind(tenant).fetch_one(&mut *lock).await?;
        let task = control(&fixture,"activate",json!({"revision":first,"expected_generation":1}));
        tokio::pin!(task);
        tokio::select! {
            result = &mut task => anyhow::bail!("提交没有等待行锁: {result:?}"),
            result = async {
                tokio::time::timeout(Duration::from_secs(3),async {
                    loop {
                        let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'UPDATE runtime_enhancement_heads%')").fetch_one(pool.as_ref()).await?;
                        if waiting { break Ok::<_,anyhow::Error>(()); }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await?
            } => result?,
        }
        fixture.identity.management_allowed.store(false,Ordering::SeqCst);
        lock.rollback().await?;
        assert!(task.await.is_err());
        assert_eq!(service.head(tenant).await?.generation,1);
        let events: i64 = sqlx::query_scalar("SELECT count(*) FROM runtime_enhancement_events WHERE tenant_id=$1").bind(tenant).fetch_one(pool.as_ref()).await?;
        assert_eq!(events,1,"撤权后的事务不能留下指针或审计提交");
        Ok::<_,anyhow::Error>(())
    }).await;
    let stopped = fixture.state.shutdown().await;
    fixture.server.abort();
    outcome??;
    stopped
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn evolution_plugin_is_optional_and_cannot_bootstrap_missing_mechanisms() -> Result<()> {
    let fixture = Fixture::with_plugins(vec![Arc::new(GeneratorPlugin(Arc::new(
        Generator::default(),
    )))])
    .await?;
    let result = fixture.publish(fixture::package(Language::Rhai)?, 0).await;
    assert!(result.is_err());
    fixture.state.shutdown().await?;
    fixture.server.abort();
    Ok(())
}
