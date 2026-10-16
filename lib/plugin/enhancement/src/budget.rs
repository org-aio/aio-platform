use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};

const MAX_CALLS: usize = 64;
const MAX_TRANSFER_BYTES: usize = 1024 * 1024;
const WALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 脚本及异步能力共享的截止时间与取消状态，不含可重新进入脚本的接口。
#[derive(Clone)]
pub struct InvocationControl {
    budget: Arc<Budget>,
}

struct Budget {
    started: Instant,
    deadline: Instant,
    waiting: Mutex<Waiting>,
    remaining_calls: AtomicUsize,
    remaining_bytes: AtomicUsize,
    cancelled: AtomicBool,
    exhausted: AtomicBool,
}

#[derive(Default)]
struct Waiting {
    elapsed: Duration,
    started: Option<Instant>,
}

pub(crate) struct CapabilityWait {
    control: InvocationControl,
}

impl InvocationControl {
    pub(crate) fn new(deadline: Option<Instant>) -> Self {
        let started = Instant::now();
        Self {
            budget: Arc::new(Budget {
                started,
                deadline: deadline
                    .unwrap_or(started + WALL_TIMEOUT)
                    .min(started + WALL_TIMEOUT),
                waiting: Mutex::default(),
                remaining_calls: AtomicUsize::new(MAX_CALLS),
                remaining_bytes: AtomicUsize::new(MAX_TRANSFER_BYTES),
                cancelled: AtomicBool::new(false),
                exhausted: AtomicBool::new(false),
            }),
        }
    }

    /// 能力适配器须在此期限内取消等待；不能为子调用重置时钟。
    pub fn deadline(&self) -> Instant {
        self.budget.deadline
    }

    pub(crate) fn cancel(&self) {
        self.budget.cancelled.store(true, Ordering::Relaxed);
    }

    /// 等待宿主 I/O 不消耗脚本计算时间，仍受整条链的 30 秒期限约束。
    pub fn check(&self) -> Result<()> {
        ensure!(
            !self.budget.cancelled.load(Ordering::Relaxed),
            "增强调用已取消"
        );
        ensure!(
            !self.budget.exhausted.load(Ordering::Relaxed),
            "增强调用链预算已耗尽"
        );
        let now = Instant::now();
        ensure!(now < self.budget.deadline, "增强调用链等待超时");
        let waiting = self
            .budget
            .waiting
            .lock()
            .map_err(|_| anyhow::anyhow!("增强时限锁损坏"))?;
        let io = waiting.elapsed
            + waiting
                .started
                .map(|start| now.saturating_duration_since(start))
                .unwrap_or_default();
        ensure!(
            (now - self.budget.started).saturating_sub(io) < crate::EXECUTION_TIMEOUT,
            "增强调用链执行超时"
        );
        Ok(())
    }

    pub(crate) fn wait(&self) -> Result<CapabilityWait> {
        self.check()?;
        let mut waiting = self
            .budget
            .waiting
            .lock()
            .map_err(|_| anyhow::anyhow!("增强时限锁损坏"))?;
        ensure!(
            waiting.started.is_none(),
            "同一脚本调用链不能并行等待宿主能力"
        );
        waiting.started = Some(Instant::now());
        Ok(CapabilityWait {
            control: self.clone(),
        })
    }

    pub(crate) fn call(&self) -> Result<()> {
        self.consume(&self.budget.remaining_calls, 1)
    }

    pub(crate) fn transfer(&self, bytes: usize) -> Result<()> {
        self.consume(&self.budget.remaining_bytes, bytes)
    }

    fn consume(&self, remaining: &AtomicUsize, amount: usize) -> Result<()> {
        if remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(amount)
            })
            .is_err()
        {
            self.budget.exhausted.store(true, Ordering::Relaxed);
            anyhow::bail!("增强调用链预算已耗尽");
        }
        Ok(())
    }
}

impl Drop for CapabilityWait {
    fn drop(&mut self) {
        match self.control.budget.waiting.lock() {
            Ok(mut waiting) => {
                if let Some(started) = waiting.started.take() {
                    waiting.elapsed += started.elapsed();
                }
            }
            Err(_) => self.control.budget.exhausted.store(true, Ordering::Relaxed),
        }
    }
}
