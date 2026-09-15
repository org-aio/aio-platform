#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use anyhow::{Context as _, Result, ensure};
use std::{fs, path::Path, process::Command};

#[cfg(target_os = "linux")]
use linux as system;
#[cfg(target_os = "macos")]
use macos as system;
#[cfg(windows)]
use windows as system;

pub fn register(executable: &Path) -> Result<()> {
    let root = crate::install::helper_root()?;
    fs::create_dir_all(&root)?;
    let binary = root.join(if cfg!(windows) { "aio.exe" } else { "aio" });
    if executable != binary {
        let temporary = root.join("aio-new");
        fs::copy(executable, &temporary).context("复制本机助手失败")?;
        fs::rename(temporary, &binary)?;
    }
    system::register(&binary)?;
    println!("已注册 aio:// 本机助手；网页安装会在终端显示计划并等待确认。");
    Ok(())
}

pub fn unregister() -> Result<()> {
    system::unregister()?;
    // 保留独立二进制和工具记录，Windows 不允许删除当前正在运行的程序。
    println!("已解除 aio:// 注册，工具安装记录保留。卸载工具请使用 aio tool uninstall <id>。");
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status().context("运行系统协议注册命令失败")?;
    ensure!(status.success(), "系统协议注册命令失败: {status}");
    Ok(())
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
