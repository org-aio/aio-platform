//! 宿主进程承载与健康判定。
//!
//! 角色分工：
//! - `boot` 在承载服务内运行，只校验当前发布后 `exec` 替换进程映像，因此 PID、
//!   stdout 与 systemd 生命周期都保持不变。
//! - `activate` 由部署侧一次性执行（SSH、CI 或运维命令），它调用 `restart` 让
//!   systemd 用新发布重新执行 `boot`，然后轮询健康并在失败时回滚。
//!
//! 因为 `activate` 不在服务进程内，`restart` 允许是阻塞的系统调用；若误在服务内
//! 调用，`activate` 会通过 `AIO_LAUNCHER_SERVICE=1` 直接拒绝，避免自杀式重启。

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};
use async_trait::async_trait;

/// 承载宿主的进程控制。实现必须让宿主的 stdout 继续流向调用方日志。
#[async_trait]
pub trait HostSupervisor: Send + Sync {
    /// 用目标发布重启承载服务；返回后不保证已经健康。
    async fn restart(&self, release: &Path) -> Result<()>;
    /// 轮询健康入口，失败信息保留最后一次原因。
    async fn wait_healthy(&self, release: &Path) -> Result<()>;
    /// 把当前进程替换为目标发布的宿主；成功时永不返回。
    async fn exec(&self, release: &Path) -> Result<()>;
}

/// systemd 承载：`restart` 调用部署方配置的命令，`exec` 直接替换进程映像。
pub struct SystemdSupervisor {
    pub binary_name: String,
    pub health_url: String,
    pub health_timeout: Duration,
    pub restart_command: Vec<String>,
    client: reqwest::Client,
}

impl SystemdSupervisor {
    /// 从环境变量构造；`AIO_LAUNCHER_RESTART` 用空格分隔命令与参数。
    pub fn from_env() -> Result<Self> {
        let binary_name =
            std::env::var("AIO_LAUNCHER_BINARY").unwrap_or_else(|_| "aio-idea".into());
        let health_url = std::env::var("AIO_LAUNCHER_HEALTH_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3080/health".into());
        let seconds: u64 = std::env::var("AIO_LAUNCHER_HEALTH_SECONDS")
            .unwrap_or_else(|_| "300".into())
            .parse()
            .context("AIO_LAUNCHER_HEALTH_SECONDS 必须是整数")?;
        let restart_command = std::env::var("AIO_LAUNCHER_RESTART")
            .unwrap_or_else(|_| "systemctl restart aio-idea.service".into())
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        ensure!(!restart_command.is_empty(), "AIO_LAUNCHER_RESTART 不能为空");
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .context("创建健康探测客户端失败")?;
        Ok(Self {
            binary_name,
            health_url,
            health_timeout: Duration::from_secs(seconds),
            restart_command,
            client,
        })
    }
}

#[async_trait]
impl HostSupervisor for SystemdSupervisor {
    async fn restart(&self, release: &Path) -> Result<()> {
        // 先确认候选真的可以被承载，避免让 systemd 去启动一个不存在的路径。
        host_binary(release, &self.binary_name)?;
        let (program, arguments) = self
            .restart_command
            .split_first()
            .context("重启命令缺少程序名")?;
        // `activate` 已把 `current` 原子指向目标发布；重启命令只负责让 systemd
        // 重新执行 `boot`。不要把发布路径追加给 `systemctl` 等不接受额外参数的
        // 命令，否则重启会被当成未知单元而失败。
        let status = Command::new(program)
            .args(arguments)
            .status()
            .with_context(|| format!("执行重启命令失败: {program}"))?;
        ensure!(status.success(), "重启命令返回失败: {status}");
        Ok(())
    }

    async fn wait_healthy(&self, release: &Path) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.health_timeout;
        let mut last_error = String::from("尚未探测");
        while tokio::time::Instant::now() < deadline {
            match self.client.get(&self.health_url).send().await {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => last_error = format!("HTTP {}", response.status()),
                Err(error) => last_error = error.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        bail!(
            "发布 {} 未在 {:?} 内通过健康检查: {last_error}",
            release.display(),
            self.health_timeout
        )
    }

    async fn exec(&self, release: &Path) -> Result<()> {
        let binary = host_binary(release, &self.binary_name)?;
        // exec 成功后本进程映像被宿主替换：PID、日志与 systemd 生命周期都保持不变。
        let error =
            std::os::unix::process::CommandExt::exec(Command::new(&binary).current_dir(release));
        Err(error).with_context(|| format!("替换为宿主进程失败: {}", binary.display()))
    }
}

fn host_binary(release: &Path, binary_name: &str) -> Result<PathBuf> {
    let binary = release.join(binary_name);
    ensure!(
        binary.is_file(),
        "发布目录缺少宿主二进制: {}",
        binary.display()
    );
    Ok(binary)
}
