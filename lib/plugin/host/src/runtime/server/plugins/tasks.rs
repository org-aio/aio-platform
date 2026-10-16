//! 插件拥有的后台任务；停止时关闭入口、协作取消并等待已有变更完成。

use std::{future::Future, sync::Mutex};

use anyhow::{Result, ensure};
use tokio::task::{JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

/// 启动动作与请求处理共享的任务作用域，不能在停止后继续派生工作。
pub struct PluginTasks {
    cancellation: CancellationToken,
    state: Mutex<TaskState>,
}

#[derive(Default)]
struct TaskState {
    closed: bool,
    jobs: JoinSet<Result<()>>,
    errors: Vec<String>,
    failures: usize,
}

impl PluginTasks {
    pub(super) fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            state: Mutex::new(TaskState::default()),
        }
    }

    /// 在安全边界检查取消；已经开始的持久化变更应先完成提交或补偿。
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// 收容任务直至完成；任务必须响应取消，或者是能够自行结束的有限工作。
    pub fn spawn(&self, work: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("插件任务锁已损坏"))?;
        ensure!(
            !state.closed && !self.cancellation.is_cancelled(),
            "插件正在停止"
        );
        while let Some(result) = state.jobs.try_join_next() {
            state.record(result);
        }
        state.jobs.spawn(work);
        Ok(())
    }

    pub(super) async fn drain(&self) -> Result<()> {
        self.cancellation.cancel();
        let mut jobs = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("插件任务锁已损坏"))?;
            state.closed = true;
            std::mem::take(&mut state.jobs)
        };
        while let Some(result) = jobs.join_next().await {
            self.state
                .lock()
                .map_err(|_| anyhow::anyhow!("插件任务锁已损坏"))?
                .record(result);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("插件任务锁已损坏"))?;
        ensure!(
            state.failures == 0,
            "{} 个后台任务失败: {}",
            state.failures,
            state.errors.join("; ")
        );
        Ok(())
    }
}

impl TaskState {
    fn record(&mut self, result: std::result::Result<Result<()>, JoinError>) {
        let error = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => format!("{error:#}"),
            Err(error) => format!("后台任务未正常结束: {error}"),
        };
        eprintln!("插件后台任务失败: {error}");
        self.failures += 1;
        // 保留总数和有限样本，避免长寿命插件的错误历史无限增长。
        if self.errors.len() < 16 {
            self.errors.push(error);
        }
    }
}
