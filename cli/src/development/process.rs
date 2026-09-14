use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::Path, process::Stdio};
use tokio::process::{Child, Command};

pub(super) struct OwnedProcess {
    pub child: Child,
    group: u32,
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            signal(self.group, "-TERM");
            // JVM/构建器可能再派生子进程；按本次创建的进程组回收，并给容器适配器清理容器和临时卷的退出时间。
            for _ in 0..250 {
                let _ = self.child.try_wait();
                if !signal(self.group, "-0") {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            signal(self.group, "-KILL");
        }
        let _ = self.child.start_kill();
    }
}

#[cfg(unix)]
fn signal(group: u32, signal: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args([signal, &format!("-{group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn spawn(
    argv: &[String],
    root: &Path,
    env: &BTreeMap<String, String>,
    log: &Path,
) -> Result<OwnedProcess> {
    let (program, args) = argv.split_first().context("命令为空")?;
    let output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    let mut command = Command::new(program);
    if let Some(directory) = std::env::current_exe()?.parent() {
        let mut paths = vec![directory.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        command.env("PATH", std::env::join_paths(paths)?);
    }
    command
        .args(args)
        .current_dir(root)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .with_context(|| format!("启动失败: {program}"))?;
    let group = child.id().context("启动进程未返回 PID")?;
    Ok(OwnedProcess { child, group })
}

pub(super) async fn build(argv: &[String], root: &Path, log: &Path) -> Result<()> {
    println!("构建 {}: {}", root.display(), argv.join(" "));
    let mut process = spawn(argv, root, &BTreeMap::new(), log)?;
    let result = process.child.wait().await?;
    ensure!(
        result.success(),
        "构建失败 ({result})，日志: {}",
        log.display()
    );
    Ok(())
}

pub(super) fn port() -> Result<u16> {
    Ok(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port(),
    )
}

pub(super) fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}
