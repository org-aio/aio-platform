use super::*;
use anyhow::{Context, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

pub async fn exercise(
    left: Arc<dyn EnhancementCatalog>,
    right: Arc<dyn EnhancementCatalog>,
) -> Result<()> {
    let tenant = uuid::Uuid::new_v4().to_string();
    let first = package(Language::Rhai);
    let revision = first.revision()?;
    // 取得写锁之后拒绝授权，不得留下候选或缓存成功结果。
    assert!(
        left.propose(
            &tenant,
            "revoked",
            &first,
            Box::pin(async { anyhow::bail!("已撤权") })
        )
        .await
        .is_err()
    );
    assert!(right.revisions(&tenant).await?.is_empty());
    left.propose(&tenant, "author", &first, authorize()).await?;
    right
        .propose(&tenant, "replacement", &first, authorize())
        .await?;
    assert_eq!(right.revisions(&tenant).await?[0].author, "author");
    assert!(right.package("foreign", &revision).await.is_err());
    let request = Activation {
        revision: Some(revision.clone()),
        expected_generation: 0,
    };
    assert!(
        left.activate(&tenant, "actor", &request, true, authorize())
            .await
            .is_err()
    );
    assert_eq!(right.head(&tenant).await?, Head::default());
    assert!(right.history(&tenant).await?.is_empty());

    // 第一笔事务占有租户写锁，第二笔必须等锁后再次检查授权。
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let candidate = left.clone();
    let candidate_tenant = tenant.clone();
    let candidate_request = request.clone();
    let acquired = entered.clone();
    let release = resume.clone();
    let pending = tokio::spawn(async move {
        candidate
            .activate(
                &candidate_tenant,
                "first",
                &candidate_request,
                false,
                Box::pin(async move {
                    acquired.notify_one();
                    release.notified().await;
                    Ok(())
                }),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let candidate = right.clone();
    let candidate_tenant = tenant.clone();
    let candidate_revision = revision.clone();
    let mut blocked = tokio::spawn(async move {
        candidate
            .activate(
                &candidate_tenant,
                "revoked",
                &Activation {
                    revision: Some(candidate_revision),
                    expected_generation: 1,
                },
                false,
                Box::pin(async move {
                    observed.store(true, Ordering::SeqCst);
                    anyhow::bail!("等待写锁时撤权")
                }),
            )
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut blocked)
            .await
            .is_err()
    );
    assert!(!called.load(Ordering::SeqCst));
    resume.notify_one();
    assert_eq!(pending.await??.context("首次激活失败")?.generation, 1);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), blocked)
            .await??
            .is_err()
    );
    assert!(called.load(Ordering::SeqCst));
    assert_eq!(left.head(&tenant).await?.generation, 1);
    assert_eq!(right.history(&tenant).await?.len(), 1);
    assert!(
        left.activate(&tenant, "stale", &request, false, authorize())
            .await?
            .is_none()
    );

    // 请求被丢弃时，已暂存的指针和审计同样回滚。
    let candidate = right.clone();
    let candidate_tenant = tenant.clone();
    let acquired = entered.clone();
    let pending = tokio::spawn(async move {
        candidate
            .activate(
                &candidate_tenant,
                "cancelled",
                &Activation {
                    revision: None,
                    expected_generation: 1,
                },
                false,
                Box::pin(async move {
                    acquired.notify_one();
                    std::future::pending::<Result<()>>().await
                }),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
    pending.abort();
    assert!(pending.await.err().context("提交未取消")?.is_cancelled());
    assert_eq!(left.head(&tenant).await?.generation, 1);
    assert_eq!(left.history(&tenant).await?.len(), 1);

    let change = Activation {
        revision: None,
        expected_generation: 1,
    };
    let (a, b) = tokio::join!(
        left.activate(&tenant, "a", &change, false, authorize()),
        right.activate(&tenant, "b", &change, false, authorize())
    );
    assert_eq!(usize::from(a?.is_some()) + usize::from(b?.is_some()), 1);
    let mut changed = first.clone();
    changed.enhancements[0].input_schema = Some(json!({"type":"integer"}).try_into()?);
    right
        .propose(&tenant, "author", &changed, authorize())
        .await?;
    assert!(
        left.activate(
            &tenant,
            "actor",
            &Activation {
                revision: Some(changed.revision()?),
                expected_generation: 2
            },
            false,
            authorize()
        )
        .await
        .is_err()
    );
    assert_eq!(right.head(&tenant).await?.generation, 2);
    assert_eq!(right.history(&tenant).await?.len(), 2);
    right
        .activate(
            &tenant,
            "actor",
            &Activation {
                revision: Some(revision.clone()),
                expected_generation: 2,
            },
            true,
            authorize(),
        )
        .await?
        .context("回滚失败")?;
    left.started(&tenant, &revision, 1, PATH).await?;
    right.finished(&tenant, 1, PATH, true).await?;
    left.started(&tenant, &revision, 3, PATH).await?;
    right.finished(&tenant, 3, PATH, false).await?;
    assert_eq!(left.observe(&tenant, &revision, 1).await?[0].succeeded, 1);
    assert_eq!(right.observe(&tenant, &revision, 3).await?[0].failed, 1);
    assert!(left.observe("foreign", &revision, 1).await.is_err());
    assert!(left.observe(&tenant, &revision, 2).await.is_err());
    Ok(())
}
