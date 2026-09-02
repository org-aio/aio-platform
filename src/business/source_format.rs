use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, ensure};

pub(super) fn write_rust_source_if_changed(path: &Path, source: &str) -> Result<bool> {
    if fs::read(path).is_ok_and(|current| current == source.as_bytes()) {
        return Ok(false);
    }
    let source = format_rust_source(source)?;
    super::write_if_changed(path, &source)
}

fn format_rust_source(source: &str) -> Result<Vec<u8>> {
    let mut child = match Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(source.as_bytes().to_vec()),
        Err(error) => return Err(error).context("启动 rustfmt 失败"),
    };
    child
        .stdin
        .take()
        .context("获取 rustfmt 标准输入失败")?
        .write_all(source.as_bytes())
        .context("向 rustfmt 写入生成源码失败")?;
    let output = child
        .wait_with_output()
        .context("等待 rustfmt 格式化生成源码失败")?;
    ensure!(
        output.status.success(),
        "rustfmt 格式化业务模块失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}
