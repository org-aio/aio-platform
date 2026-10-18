//! 启动器切换协议的确定性验收：成功、回滚、崩溃恢复与路径校验都使用替身承载。

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use async_trait::async_trait;
use az_plugin_launcher::journal::current_holder;
use az_plugin_launcher::{
    HostSupervisor, Journal, Layout, Phase, RELEASE_MANIFEST, ReleaseManifest, SwitchOutcome,
    SwitchRequest, activate, repair, safe_join, settle,
};
use sha2::{Digest, Sha256};

/// 记录重启顺序与不可用发布的替身。
///
/// 健康状况按发布目录判定：宿主可用性属于具体发布，回滚必须重新评估旧发布，
/// 不能被新发布的失败结果污染。
#[derive(Default)]
struct Recorder {
    restarts: Mutex<Vec<PathBuf>>,
    unhealthy: Mutex<HashSet<PathBuf>>,
    failing_restart: Mutex<HashSet<PathBuf>>,
}

impl Recorder {
    fn restarts(&self) -> Vec<PathBuf> {
        self.restarts.lock().expect("锁").clone()
    }
    fn mark_unhealthy(&self, release: &Path) {
        self.unhealthy
            .lock()
            .expect("锁")
            .insert(release.to_path_buf());
    }
    fn mark_restart_failure(&self, release: &Path) {
        self.failing_restart
            .lock()
            .expect("锁")
            .insert(release.to_path_buf());
    }
    fn healthy(&self, release: &Path) -> bool {
        !self.unhealthy.lock().expect("锁").contains(release)
    }
    fn restart_fails(&self, release: &Path) -> bool {
        self.failing_restart.lock().expect("锁").contains(release)
    }
}

struct FakeSupervisor(Arc<Recorder>);

#[async_trait]
impl HostSupervisor for FakeSupervisor {
    async fn restart(&self, release: &Path) -> Result<()> {
        self.0
            .restarts
            .lock()
            .expect("锁")
            .push(release.to_path_buf());
        if self.0.restart_fails(release) {
            bail!("systemd 拒绝启动 {}", release.display());
        }
        Ok(())
    }
    async fn wait_healthy(&self, release: &Path) -> Result<()> {
        if self.0.healthy(release) {
            Ok(())
        } else {
            bail!("候选未通过健康检查")
        }
    }
    async fn exec(&self, release: &Path) -> Result<()> {
        bail!("替身不 exec: {}", release.display())
    }
}

/// 写一个通过校验的候选发布目录：清单摘要由真实文件内容计算。
async fn candidate(layout: &Layout, revision: &str, web: &str) -> Result<PathBuf> {
    let directory = layout.release(revision);
    tokio::fs::create_dir_all(&directory).await?;
    tokio::fs::write(directory.join("aio-idea"), b"host-binary").await?;
    tokio::fs::write(directory.join("index.html"), web).await?;
    let mut extra = BTreeMap::new();
    extra.insert(
        "index.html".to_owned(),
        format!("{:x}", Sha256::digest(web.as_bytes())),
    );
    let manifest = ReleaseManifest {
        revision: revision.to_owned(),
        host_binary: "aio-idea".to_owned(),
        host_sha256: format!("{:x}", Sha256::digest(b"host-binary")),
        web_index: "index.html".to_owned(),
        extra_files: extra,
    };
    tokio::fs::write(
        directory.join(RELEASE_MANIFEST),
        serde_json::to_vec(&manifest)?,
    )
    .await?;
    Ok(directory)
}

#[tokio::test]
async fn activates_verified_candidate_then_rolls_back_on_health_failure() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    let first = candidate(&layout, &"a".repeat(40), "<html>v1</html>").await?;
    let recorder = Arc::new(Recorder::default());
    let supervisor = FakeSupervisor(Arc::clone(&recorder));

    let outcome = activate(
        &layout,
        &supervisor,
        SwitchRequest {
            candidate: first.clone(),
        },
    )
    .await?;
    assert_eq!(
        outcome,
        SwitchOutcome::Activated {
            revision: "a".repeat(40)
        }
    );
    assert_eq!(layout.current_target().await?, Some(first.clone()));
    let journal = Journal::read(&layout.journal()).await?.expect("日志");
    assert_eq!(journal.phase, Phase::Confirmed);

    // 第二次发布健康失败：必须回到第一次并保留可审计原因。
    let second = candidate(&layout, &"b".repeat(40), "<html>v2</html>").await?;
    recorder.mark_unhealthy(&second);
    let outcome = activate(&layout, &supervisor, SwitchRequest { candidate: second }).await?;
    assert!(matches!(outcome, SwitchOutcome::RolledBack { .. }));
    assert_eq!(layout.current_target().await?, Some(first.clone()));
    let journal = Journal::read(&layout.journal()).await?.expect("日志");
    assert_eq!(journal.phase, Phase::RolledBack);
    assert_eq!(journal.current, first);
    assert!(journal.detail.expect("失败原因").contains("健康"));
    // 重启顺序：候选 v1、候选 v2、回滚 v1。
    assert_eq!(recorder.restarts().len(), 3);
    Ok(())
}

#[tokio::test]
async fn rolls_back_when_service_restart_itself_fails() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    let first = candidate(&layout, &"f".repeat(40), "<html>v1</html>").await?;
    let recorder = Arc::new(Recorder::default());
    let supervisor = FakeSupervisor(Arc::clone(&recorder));
    activate(
        &layout,
        &supervisor,
        SwitchRequest {
            candidate: first.clone(),
        },
    )
    .await?;

    let second = candidate(&layout, &"1".repeat(40), "<html>v2</html>").await?;
    recorder.mark_restart_failure(&second);
    let outcome = activate(&layout, &supervisor, SwitchRequest { candidate: second }).await?;
    assert!(matches!(outcome, SwitchOutcome::RolledBack { .. }));
    assert_eq!(layout.current_target().await?, Some(first));
    let journal = Journal::read(&layout.journal()).await?.expect("日志");
    assert_eq!(journal.phase, Phase::RolledBack);
    Ok(())
}

#[tokio::test]
async fn rejects_candidate_whose_digest_does_not_match_manifest() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    let directory = candidate(&layout, &"c".repeat(40), "<html>v1</html>").await?;
    // 篡改二进制但不更新清单：必须拒绝且不切换。
    tokio::fs::write(directory.join("aio-idea"), b"tampered").await?;
    let recorder = Arc::new(Recorder::default());
    let error = activate(
        &layout,
        &FakeSupervisor(Arc::clone(&recorder)),
        SwitchRequest {
            candidate: directory,
        },
    )
    .await
    .expect_err("摘要不匹配必须失败");
    assert!(format!("{error:#}").contains("摘要不匹配"));
    assert!(layout.current_target().await?.is_none());
    assert!(recorder.restarts().is_empty());
    Ok(())
}

#[tokio::test]
async fn repairs_unconfirmed_switch_after_crash() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    let previous = candidate(&layout, &"d".repeat(40), "<html>old</html>").await?;
    let candidate_dir = candidate(&layout, &"e".repeat(40), "<html>new</html>").await?;
    // 模拟切换过程中崩溃：current 已指向候选，但日志仍是 Switching。
    layout.point_current(&candidate_dir).await?;
    Journal {
        revision: 7,
        phase: Phase::Switching,
        current: candidate_dir,
        previous: Some(previous.clone()),
        holder: None,
        detail: None,
    }
    .write(&layout.journal())
    .await?;

    let recorder = Arc::new(Recorder::default());
    let outcome = repair(&layout, &FakeSupervisor(Arc::clone(&recorder)))
        .await?
        .expect("需要修复");
    assert!(matches!(outcome, SwitchOutcome::RolledBack { .. }));
    assert_eq!(layout.current_target().await?, Some(previous));
    let journal = Journal::read(&layout.journal()).await?.expect("日志");
    assert_eq!(journal.phase, Phase::RolledBack);
    Ok(())
}

#[tokio::test]
async fn refuses_paths_outside_the_release_root() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    tokio::fs::create_dir_all(layout.root.join("releases")).await?;
    let outside = temporary.path().join("outside");
    tokio::fs::create_dir_all(&outside).await?;
    let error = layout
        .ensure_managed(&outside)
        .await
        .expect_err("发布目录必须位于 releases 之下");
    assert!(format!("{error:#}").contains("必须位于"));
    assert!(safe_join(&layout.root, "../escape").is_err());
    assert!(safe_join(&layout.root, "/absolute").is_err());
    Ok(())
}

#[tokio::test]
async fn corrupt_journal_is_rejected_instead_of_treated_as_first_start() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    tokio::fs::create_dir_all(&layout.root).await?;
    tokio::fs::write(layout.journal(), b"{").await?;
    let error = Journal::read(&layout.journal())
        .await
        .expect_err("损坏日志必须报错");
    assert!(format!("{error:#}").contains("损坏"));
    Ok(())
}

#[tokio::test]
async fn boot_waits_for_in_flight_activation_instead_of_competing() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    let previous = candidate(&layout, &"2".repeat(40), "<html>old</html>").await?;
    let candidate_dir = candidate(&layout, &"3".repeat(40), "<html>new</html>").await?;
    layout.point_current(&candidate_dir).await?;
    Journal {
        revision: 11,
        phase: Phase::Switching,
        current: candidate_dir,
        previous: Some(previous.clone()),
        holder: None,
        detail: None,
    }
    .write(&layout.journal())
    .await?;
    let recorder = Arc::new(Recorder::default());
    let supervisor = FakeSupervisor(Arc::clone(&recorder));

    // 持有者活跃：boot 必须沉默，既不回滚也不提交。
    let active = Journal {
        holder: Some(current_holder()?),
        ..Journal::read(&layout.journal()).await?.expect("日志")
    };
    active.write(&layout.journal()).await?;
    let settlement = tokio::time::timeout(
        std::time::Duration::from_millis(400),
        settle(&layout, &supervisor, std::time::Duration::from_millis(50)),
    )
    .await;
    assert!(
        settlement.is_err(),
        "持有者活跃时 settle 必须继续等待而不是抢占"
    );
    assert_eq!(
        Journal::read(&layout.journal()).await?.expect("日志").phase,
        Phase::Switching
    );
    assert!(recorder.restarts().is_empty());

    // 部署侧确认提交后，boot 应直接放行且不改写记录。
    let current = Journal::read(&layout.journal()).await?.expect("日志");
    Journal {
        phase: Phase::Confirmed,
        ..current
    }
    .write(&layout.journal())
    .await?;
    assert!(
        settle(&layout, &supervisor, std::time::Duration::from_millis(50))
            .await?
            .is_none()
    );
    assert!(recorder.restarts().is_empty());

    // 超时的在途切换说明那次 activate 已崩溃：必须回滚到上一发布。
    let stuck = Journal {
        revision: 20,
        phase: Phase::Switching,
        current: layout.release(&"4".repeat(40)),
        previous: Some(previous.clone()),
        holder: None,
        detail: None,
    };
    tokio::fs::create_dir_all(&stuck.current).await?;
    tokio::fs::write(stuck.current.join("aio-idea"), b"host-binary").await?;
    tokio::fs::write(stuck.current.join("index.html"), b"<html>x</html>").await?;
    let mut extra = BTreeMap::new();
    extra.insert(
        "index.html".to_owned(),
        format!("{:x}", Sha256::digest(b"<html>x</html>")),
    );
    let manifest = ReleaseManifest {
        revision: "4".repeat(40),
        host_binary: "aio-idea".to_owned(),
        host_sha256: format!("{:x}", Sha256::digest(b"host-binary")),
        web_index: "index.html".to_owned(),
        extra_files: extra,
    };
    tokio::fs::write(
        stuck.current.join(RELEASE_MANIFEST),
        serde_json::to_vec(&manifest)?,
    )
    .await?;
    stuck.write(&layout.journal()).await?;
    let outcome = settle(&layout, &supervisor, std::time::Duration::from_millis(50)).await?;
    assert!(matches!(outcome, Some(SwitchOutcome::RolledBack { .. })));
    assert_eq!(layout.current_target().await?, Some(previous));
    Ok(())
}

#[tokio::test]
async fn adopts_legacy_release_then_boot_stays_strict() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let layout = Layout::new(temporary.path());
    // 协议迁移前的既有发布：有二进制与前端，但没有 release-manifest.json。
    let legacy = layout.release(&"9".repeat(40));
    tokio::fs::create_dir_all(legacy.join("web")).await?;
    tokio::fs::write(legacy.join("aio-idea"), b"legacy-host").await?;
    tokio::fs::write(legacy.join("web/index.html"), b"<html>legacy</html>").await?;
    tokio::fs::write(legacy.join("aio.toml"), b"plugins = []\n").await?;
    assert!(
        ReleaseManifest::read(&legacy).await.is_err(),
        "旧发布没有清单"
    );

    let manifest = ReleaseManifest::adopt(&legacy, &"9".repeat(40)).await?;
    manifest.verify(&legacy).await?;
    tokio::fs::write(
        legacy.join(RELEASE_MANIFEST),
        serde_json::to_vec(&manifest)?,
    )
    .await?;
    // 补写后必须能被严格读取与校验，boot 不需要任何弱化路径。
    let reloaded = ReleaseManifest::read(&legacy).await?;
    reloaded.verify(&legacy).await?;
    assert_eq!(reloaded.revision, "9".repeat(40));
    assert_eq!(
        reloaded.host_sha256,
        format!("{:x}", Sha256::digest(b"legacy-host"))
    );

    // 篡改后仍会被严格拒绝。
    tokio::fs::write(legacy.join("aio-idea"), b"tampered").await?;
    assert!(reloaded.verify(&legacy).await.is_err());
    Ok(())
}
