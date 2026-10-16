//! 部署方指定的本机 MCP 进程；只负责受限管道和进程组，不重复实现能力或监督服务。

use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, future};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use rmcp::{
    RoleClient,
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::{Transport, async_rw::JsonRpcMessageCodec, sink_stream::SinkStreamTransport},
};
use serde::Deserialize;
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};
use tokio_util::io::{ReaderStream, StreamReader};

use super::configuration::{MAX_WIRE_BYTES, environment_name};

/// 参数是部署配置，环境只声明变量名映射；不继承宿主环境，也不执行 shell 拼接。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandSpec {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

pub(super) struct Command {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
}

impl CommandSpec {
    pub async fn prepare(self) -> Result<Command> {
        ensure!(
            cfg!(any(unix, windows)),
            "当前平台不支持 MCP stdio 进程隔离组"
        );
        ensure!(
            self.program.is_absolute() && self.cwd.is_absolute(),
            "MCP stdio 可执行文件与工作目录必须为绝对路径"
        );
        ensure!(
            self.args.len() <= 128 && self.env.len() <= 64,
            "MCP stdio 参数或环境变量数量超过配额"
        );
        let program = tokio::fs::canonicalize(self.program)
            .await
            .context("MCP stdio 可执行文件不存在")?;
        let cwd = tokio::fs::canonicalize(self.cwd)
            .await
            .context("MCP stdio 工作目录不存在")?;
        let metadata = tokio::fs::metadata(&program).await?;
        ensure!(metadata.is_file(), "MCP stdio 可执行路径不是文件");
        ensure!(
            tokio::fs::metadata(&cwd).await?.is_dir(),
            "MCP stdio 工作目录不是目录"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o111 != 0,
                "MCP stdio 文件不可执行"
            );
        }
        #[cfg(windows)]
        ensure!(
            program
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")),
            "MCP stdio 只接受可执行文件，不执行批处理脚本"
        );
        let mut bytes = 0;
        for arg in &self.args {
            ensure!(!arg.contains('\0'), "MCP stdio 参数包含空字节");
            bytes += arg.len();
        }
        let mut env = BTreeMap::new();
        for (target, source) in self.env {
            environment_name(&target)?;
            environment_name(&source)?;
            let value = std::env::var(&source)
                .map_err(|_| anyhow::anyhow!("MCP stdio 环境变量未配置或编码无效: {source}"))?;
            ensure!(!value.contains('\0'), "MCP stdio 环境变量包含空字节");
            bytes += target.len() + value.len();
            env.insert(target, value);
        }
        ensure!(bytes <= 64 * 1024, "MCP stdio 参数与环境总量超过 64 KiB");
        Ok(Command {
            program,
            args: self.args,
            cwd,
            env,
        })
    }
}

impl Command {
    pub fn spawn(&self) -> Result<(Process, impl Transport<RoleClient> + 'static)> {
        let mut command = CommandWrap::with_new(&self.program, |command| {
            command
                .args(&self.args)
                .current_dir(&self.cwd)
                .env_clear()
                .envs(&self.env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                // stderr 不能进入协议，也不能把上游凭据写入宿主日志。
                .stderr(Stdio::null());
        });
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(process_wrap::tokio::ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(process_wrap::tokio::JobObject);
        let child = command.spawn().context("启动 MCP stdio 进程失败")?;
        let mut process = Process {
            child: Some(child),
            invalid: Arc::new(AtomicBool::new(false)),
        };
        let child = process.child.as_mut().context("MCP stdio 进程缺失")?;
        let stdout = child.stdout().take().context("MCP stdio 缺少输出管道")?;
        let stdin = child.stdin().take().context("MCP stdio 缺少输入管道")?;
        let invalid = process.invalid.clone();
        // 同时限制整场会话的读取总量，空行、通知、CRLF 和未终止输出也占用预算。
        let budget_invalid = invalid.clone();
        let bounded = ReaderStream::new(stdout).scan(0usize, move |bytes, chunk| {
            let chunk = chunk.and_then(|chunk| {
                *bytes += chunk.len();
                if *bytes > 4 * MAX_WIRE_BYTES {
                    budget_invalid.store(true, Ordering::Release);
                    return Err(std::io::Error::other("MCP stdio 输出超过总量配额"));
                }
                Ok(chunk)
            });
            future::ready(Some(chunk))
        });
        // LinesCodec 在解析前限制单行。
        // 使用 SDK 的消息模型，避免其解码器在 debug 日志中输出原始错误行。
        let read = FramedRead::new(
            StreamReader::new(bounded),
            LinesCodec::new_with_max_length(MAX_WIRE_BYTES),
        );
        let stream = read.scan((), move |_, frame| {
            let message = frame
                .ok()
                .and_then(|line| serde_json::from_str::<ServerJsonRpcMessage>(&line).ok());
            if message.is_none() {
                invalid.store(true, Ordering::Release);
            }
            future::ready(message)
        });
        let sink = FramedWrite::new(
            stdin,
            JsonRpcMessageCodec::<ClientJsonRpcMessage>::new_with_max_length(MAX_WIRE_BYTES),
        );
        Ok((process, SinkStreamTransport::new(sink, stream)))
    }
}

/// 归属插件任务作用域；正常路径必须等待关闭，Drop 只为 panic/运行时退出兜底。
pub(super) struct Process {
    child: Option<Box<dyn ChildWrapper>>,
    invalid: Arc<AtomicBool>,
}

impl Process {
    pub async fn close(mut self) -> Result<()> {
        if let Some(child) = &mut self.child {
            terminate(child.as_mut()).context("终止 MCP stdio 进程组失败")?;
            tokio::time::timeout(Duration::from_secs(2), child.wait())
                .await
                .context("回收 MCP stdio 进程超时")?
                .context("回收 MCP stdio 进程失败")?;
            // 已回收后不再向旧进程组编号发信号，避免编号复用。
            self.child = None;
        }
        ensure!(
            !self.invalid.load(Ordering::Acquire),
            "MCP stdio 输出无效或超过配额"
        );
        Ok(())
    }
}

fn terminate(child: &mut dyn ChildWrapper) -> std::io::Result<()> {
    match child.start_kill() {
        Ok(()) => Ok(()),
        #[cfg(unix)]
        Err(error) if error.raw_os_error() == Some(nix::errno::Errno::ESRCH as i32) => Ok(()),
        Err(error) => Err(error),
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child
            && let Err(error) = terminate(child.as_mut())
        {
            eprintln!("MCP stdio 进程组兜底终止失败: {error}");
        }
    }
}
