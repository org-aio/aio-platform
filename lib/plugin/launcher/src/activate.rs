//! 发布切换：校验候选、原子切换、重启承载服务、健康确认，失败回滚。
//!
//! `activate` 由部署侧一次性执行，是切换事务的唯一所有者：它标记持有者、写
//! `Switching` 记录，健康成功才提交 `Confirmed`，失败则改回上一发布并写
//! `RolledBack`。`boot` 依靠持有者标记等待在途事务，而不是按时间猜测。

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};

use crate::{
    journal::{Holder, Journal, Phase, current_holder, holder_alive},
    layout::Layout,
    release::ReleaseManifest,
    supervisor::HostSupervisor,
};

/// 一次切换请求；`candidate` 必须是 `Layout::release` 覆盖的目录。
#[derive(Clone, Debug)]
pub struct SwitchRequest {
    pub candidate: PathBuf,
}

/// 切换结果，供 CLI 与测试断言使用。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SwitchOutcome {
    /// 候选通过校验与健康确认，成为当前发布。
    Activated { revision: String },
    /// 候选失败，已回到上一发布。
    RolledBack { revision: String, detail: String },
}

/// 校验候选并执行一次完整切换。
pub async fn activate(
    layout: &Layout,
    supervisor: &dyn HostSupervisor,
    request: SwitchRequest,
) -> Result<SwitchOutcome> {
    // 服务进程内调用会自杀式重启，因此直接拒绝并提示改用部署侧执行。
    ensure!(
        std::env::var_os("AIO_LAUNCHER_SERVICE").is_none(),
        "activate 必须在承载服务之外执行；服务内只运行 boot"
    );
    layout.ensure_managed(&request.candidate).await?;
    let manifest = ReleaseManifest::read(&request.candidate).await?;
    manifest.verify(&request.candidate).await?;
    let revision = manifest.revision.clone();

    let previous = layout.current_target().await?;
    let sequence = next_sequence(layout).await?;
    // 先落持有者与 Switching 记录：崩溃后 boot 依据持有者消失判定接管回滚。
    Journal {
        revision: sequence,
        phase: Phase::Switching,
        current: request.candidate.clone(),
        previous: previous.clone(),
        holder: Some(current_holder()?),
        detail: None,
    }
    .write(&layout.journal())
    .await?;

    layout.point_current(&request.candidate).await?;
    if let Err(error) = restart_and_check(supervisor, &request.candidate).await {
        let Some(previous) = previous else {
            bail!("首次发布未能通过健康检查且无可回滚版本: {error:#}");
        };
        return rollback(
            layout,
            supervisor,
            sequence,
            previous,
            revision,
            error.to_string(),
        )
        .await;
    }
    Journal {
        revision: sequence + 1,
        phase: Phase::Confirmed,
        current: request.candidate.clone(),
        previous,
        holder: None,
        detail: None,
    }
    .write(&layout.journal())
    .await?;
    Ok(SwitchOutcome::Activated { revision })
}

/// 修复未确认的切换：把 `Switching` 记录回滚到上一发布。
pub async fn repair(
    layout: &Layout,
    supervisor: &dyn HostSupervisor,
) -> Result<Option<SwitchOutcome>> {
    let Some(journal) = Journal::read(&layout.journal()).await? else {
        return Ok(None);
    };
    if journal.phase != Phase::Switching {
        return Ok(None);
    }
    let revision = journal
        .current
        .file_name()
        .context("切换记录缺少发布目录名")?
        .to_string_lossy()
        .into_owned();
    let Some(previous) = journal.previous.clone() else {
        bail!("首次发布未确认完成且无可回滚版本，请人工确认 {revision}");
    };
    layout.ensure_managed(&previous).await?;
    // 旧发布可能先于启动器协议存在，缺少清单不构成拒绝回滚的理由：回滚只要求
    // 目标可被承载并健康，清单校验只属于切换前的候选检查。
    layout.point_current(&previous).await?;
    restart_and_check(supervisor, &previous).await?;
    Journal {
        revision: journal.revision + 1,
        phase: Phase::RolledBack,
        current: previous,
        previous: None,
        holder: None,
        detail: Some("存在未确认的切换记录，已回滚".into()),
    }
    .write(&layout.journal())
    .await?;
    Ok(Some(SwitchOutcome::RolledBack {
        revision,
        detail: "未确认的切换已回滚".into(),
    }))
}

/// 等待在途切换收敛；只有持有者消失并超过宽限期才接管回滚。
///
/// 持有者仍在运行时 `boot` 必须保持沉默：那次 activate 可能正处在健康检查或
/// 回滚途中，抢先接管会破坏“成功或完全回滚”的事务语义。
pub async fn settle(
    layout: &Layout,
    supervisor: &dyn HostSupervisor,
    grace: Duration,
) -> Result<Option<SwitchOutcome>> {
    let mut orphaned: Option<(u64, tokio::time::Instant)> = None;
    loop {
        let Some(journal) = Journal::read(&layout.journal()).await? else {
            return Ok(None);
        };
        if journal.phase != Phase::Switching {
            return Ok(None);
        }
        let alive = journal.holder.as_ref().is_some_and(holder_alive);
        if alive {
            // 持有者仍活跃：重新计时，等它自己提交或回滚。
            orphaned = None;
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        }
        let entry = orphaned.get_or_insert_with(|| (journal.revision, tokio::time::Instant::now()));
        // 序号变化说明又出现了新的切换事务，重新评估。
        if entry.0 != journal.revision {
            orphaned = Some((journal.revision, tokio::time::Instant::now()));
            continue;
        }
        if entry.1.elapsed() >= grace {
            return repair(layout, supervisor).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 重启承载服务并等待目标发布健康；任一步失败都返回原因供上层回滚。
///
/// 该函数刻意不校验发布清单：回滚目标可能是没有 `release-manifest.json` 的旧发布，
/// 而回滚的可用性优先于协议一致性。清单校验只发生在候选进入切换之前。
async fn restart_and_check(supervisor: &dyn HostSupervisor, release: &Path) -> Result<()> {
    supervisor.restart(release).await?;
    supervisor.wait_healthy(release).await
}

async fn rollback(
    layout: &Layout,
    supervisor: &dyn HostSupervisor,
    sequence: u64,
    previous: PathBuf,
    revision: String,
    detail: String,
) -> Result<SwitchOutcome> {
    layout.point_current(&previous).await?;
    restart_and_check(supervisor, &previous).await?;
    Journal {
        revision: sequence + 1,
        phase: Phase::RolledBack,
        current: previous,
        previous: None,
        holder: None,
        detail: Some(detail.clone()),
    }
    .write(&layout.journal())
    .await?;
    Ok(SwitchOutcome::RolledBack { revision, detail })
}

/// 读取日志序号并递增，保证切换与回滚记录单调递增。
async fn next_sequence(layout: &Layout) -> Result<u64> {
    let journal = Journal::read(&layout.journal()).await?;
    let sequence = journal.as_ref().map_or(1, |value| value.revision + 1);
    ensure!(sequence > 0, "切换序号必须大于零");
    Ok(sequence)
}

/// 供测试与诊断使用：判断一个持有者是否仍在运行。
pub fn holder_running(holder: &Holder) -> bool {
    holder_alive(holder)
}
