//! 启动器命令行：参数解析与三个入口的编排。

use std::{path::PathBuf, time::Duration};

use crate::{
    HostSupervisor, Layout, ReleaseManifest, SwitchRequest, SystemdSupervisor, activate, repair,
    settle,
};
use anyhow::{Context as _, Result, bail};

/// 命令行入口，供平台自带二进制与产品二进制共用。
pub async fn run_cli() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let reference: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let layout = layout()?;
    let supervisor = SystemdSupervisor::from_env()?;
    match reference.as_slice() {
        ["boot"] => boot(&layout, &supervisor).await,
        ["activate", revision] => run_activate(&layout, &supervisor, revision).await,
        ["repair"] => run_repair(&layout, &supervisor).await,
        _ => bail!(
            "用法: aio-launcher [boot | activate <40 位 Git SHA> | repair]\n\
             环境: AIO_LAUNCHER_ROOT、AIO_LAUNCHER_BINARY、AIO_LAUNCHER_HEALTH_URL"
        ),
    }
}

fn layout() -> Result<Layout> {
    let root = std::env::var_os("AIO_LAUNCHER_ROOT")
        .map(PathBuf::from)
        .context("必须显式配置 AIO_LAUNCHER_ROOT")?;
    Ok(Layout::new(root))
}

/// 服务入口：等待在途切换收敛，校验当前发布后 exec 成宿主。
async fn boot(layout: &Layout, supervisor: &dyn HostSupervisor) -> Result<()> {
    // 宽限期覆盖部署侧 activate 的健康超时，再加上重启与排空的余量。持有者仍
    // 活跃时 boot 会一直等待，不因为时间流逝就抢占事务。
    let wait: u64 = std::env::var("AIO_LAUNCHER_SETTLE_SECONDS")
        .unwrap_or_else(|_| "330".into())
        .parse()
        .context("AIO_LAUNCHER_SETTLE_SECONDS 必须是整数")?;
    if let Some(outcome) = settle(layout, supervisor, Duration::from_secs(wait)).await? {
        eprintln!("启动器已修复未确认的切换: {outcome:?}");
    }
    let current = layout
        .current_target()
        .await?
        .context("没有可启动的发布：请先在部署侧运行 activate")?;
    layout.ensure_managed(&current).await?;
    let manifest = ReleaseManifest::read(&current).await?;
    manifest.verify(&current).await?;
    // 成功时进程映像被宿主替换，永不返回；PID 与日志由 systemd 保持不变。
    supervisor.exec(&current).await
}

async fn run_activate(
    layout: &Layout,
    supervisor: &dyn HostSupervisor,
    revision: &str,
) -> Result<()> {
    let candidate = layout.release(revision);
    let outcome = activate(layout, supervisor, SwitchRequest { candidate }).await?;
    println!("{outcome:?}");
    Ok(())
}

async fn run_repair(layout: &Layout, supervisor: &dyn HostSupervisor) -> Result<()> {
    match repair(layout, supervisor).await? {
        Some(outcome) => println!("{outcome:?}"),
        None => println!("无需修复"),
    }
    Ok(())
}
