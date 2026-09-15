use super::run;
use anyhow::{Context as _, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn desktop() -> Result<PathBuf> {
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .context("无法定位桌面应用目录")?;
    Ok(root.join("applications/aio-helper.desktop"))
}
pub(super) fn register(binary: &Path) -> Result<()> {
    let desktop = desktop()?;
    fs::create_dir_all(desktop.parent().context("桌面应用目录无效")?)?;
    let binary = binary.to_str().context("助手路径不是 UTF-8")?;
    anyhow::ensure!(
        !binary.contains(['\n', '\r', '%', '`', '$']),
        "助手路径含 desktop entry 不支持的字符"
    );
    let escaped = binary.replace('\\', "\\\\\\\\").replace('"', "\\\\\"");
    fs::write(
        &desktop,
        format!(
            "[Desktop Entry]\nType=Application\nName=AIO Helper\nExec=\"{escaped}\" open %u\nTerminal=true\nNoDisplay=true\nMimeType=x-scheme-handler/aio;\n"
        ),
    )?;
    run(Command::new("xdg-mime").args(["default", "aio-helper.desktop", "x-scheme-handler/aio"]))
}
pub(super) fn unregister() -> Result<()> {
    let path = desktop()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
