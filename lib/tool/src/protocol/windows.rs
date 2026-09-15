use super::run;
use anyhow::{Context as _, Result, ensure};
use std::{path::Path, process::Command};

const KEY: &str = r"HKCU\Software\Classes\aio";
pub(super) fn register(binary: &Path) -> Result<()> {
    let path = binary.to_str().context("助手路径不是 UTF-8")?;
    ensure!(!path.contains('"'), "助手路径无效");
    run(Command::new("reg").args(["add", KEY, "/ve", "/d", "URL:AIO Helper", "/f"]))?;
    run(Command::new("reg").args(["add", KEY, "/v", "URL Protocol", "/d", "", "/f"]))?;
    run(Command::new("reg").args([
        "add",
        &format!(r"{KEY}\shell\open\command"),
        "/ve",
        "/d",
        &format!("\"{path}\" open \"%1\""),
        "/f",
    ]))
}
pub(super) fn unregister() -> Result<()> {
    if Command::new("reg")
        .args(["query", KEY])
        .output()?
        .status
        .success()
    {
        run(Command::new("reg").args(["delete", KEY, "/f"]))?;
    }
    Ok(())
}
