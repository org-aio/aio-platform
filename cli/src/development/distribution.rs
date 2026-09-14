use anyhow::{Context, Result, ensure};
use std::path::PathBuf;

pub(super) fn host() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let host = std::env::var_os("AIO_DEV_HOST")
        .map(PathBuf::from)
        .unwrap_or(
            executable
                .parent()
                .context("CLI 安装路径无效")?
                .join("aio-host"),
        );
    ensure!(
        host.is_file(),
        "CLI 安装缺少配套 aio-host；请安装包含开发宿主的完整 CLI 分发。源码验收可设置 AIO_DEV_HOST"
    );
    let output = std::process::Command::new(&host)
        .arg("--version")
        .output()?;
    ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).trim()
                == format!("aio-host {}", az_plugin_development::HOST_VERSION),
        "CLI 与开发宿主版本不一致"
    );
    Ok(host)
}
