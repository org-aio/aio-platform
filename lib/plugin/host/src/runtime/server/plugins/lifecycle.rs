//! 插件启动、失败补偿与关闭；异步清理由独立任务持有，调用方取消等待不会打断清理。

use std::{any::TypeId, panic::AssertUnwindSafe, sync::Arc};

use anyhow::{Context, Result, ensure};
use futures_util::FutureExt;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use super::{LifecycleHook, PluginTasks, RuntimeState};

pub(super) struct HookPlan {
    pub owner: TypeId,
    pub initialize: Vec<LifecycleHook>,
    pub start: Vec<LifecycleHook>,
    pub ready: Vec<LifecycleHook>,
    pub stop: Vec<LifecycleHook>,
}

struct Entry {
    plan: HookPlan,
    tasks: Arc<PluginTasks>,
}

pub(super) struct Lifecycle {
    entries: Vec<Entry>,
    pub cancellation: CancellationToken,
    pub requests: Arc<RwLock<()>>,
    state: Mutex<LifecycleState>,
}

#[derive(Default)]
struct LifecycleState {
    started: bool,
    entered: Vec<usize>,
    stopped: Option<Vec<String>>,
}

impl Lifecycle {
    pub fn new(plans: Vec<HookPlan>) -> Self {
        let cancellation = CancellationToken::new();
        let entries = plans
            .into_iter()
            .map(|plan| Entry {
                plan,
                tasks: Arc::new(PluginTasks::new(cancellation.child_token())),
            })
            .collect();
        Self {
            entries,
            cancellation,
            requests: Arc::new(RwLock::new(())),
            state: Mutex::new(LifecycleState::default()),
        }
    }

    pub fn tasks(&self, owner: TypeId) -> Result<Arc<PluginTasks>> {
        self.entries
            .iter()
            .find(|entry| entry.plan.owner == owner)
            .map(|entry| entry.tasks.clone())
            .context("插件未声明生命周期")
    }

    pub async fn start(state: &RuntimeState, run_hooks: bool) -> Result<()> {
        let mut guard = StartupGuard(Some(state.clone()));
        let owned = state.clone();
        let result = tokio::spawn(async move {
            owned
                .extensions
                .lifecycle
                .start_inner(&owned, run_hooks)
                .await
        })
        .await
        .context("插件启动任务未正常结束")?;
        // 启动成功后才交出运行时；失败路径在工作任务中已经完成补偿。
        guard.0 = None;
        result
    }

    async fn start_inner(&self, runtime: &RuntimeState, run_hooks: bool) -> Result<()> {
        let mut state = self.state.lock().await;
        ensure!(
            !state.started && state.stopped.is_none(),
            "运行时不能重复启动"
        );
        state.started = true;
        // 服务工厂也可能 panic；把它与钩子失败一样处理，返回前完成资源补偿。
        let result = AssertUnwindSafe(self.start_hooks(runtime, run_hooks, &mut state))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("插件初始化服务时 panic")));
        if let Err(error) = result {
            return match self.stop_locked(runtime, &mut state).await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error.context(format!("启动失败且资源清理失败: {cleanup:#}"))),
            };
        }
        Ok(())
    }

    async fn start_hooks(
        &self,
        runtime: &RuntimeState,
        run_hooks: bool,
        state: &mut LifecycleState,
    ) -> Result<()> {
        for (index, entry) in self.entries.iter().enumerate() {
            self.check_cancelled()?;
            // 先登记补偿，即使当前插件只完成了一半，停止动作也会执行。
            state.entered.push(index);
            for hook in &entry.plan.initialize {
                self.check_cancelled()?;
                invoke(*hook, runtime, entry, "初始化").await?;
            }
        }
        for resolve in &runtime.extensions.requirements {
            self.check_cancelled()?;
            resolve(&runtime.extensions.services, true)?;
        }
        if run_hooks {
            for entry in &self.entries {
                for hook in &entry.plan.start {
                    self.check_cancelled()?;
                    invoke(*hook, runtime, entry, "启动").await?;
                }
            }
        }
        if run_hooks {
            for entry in &self.entries {
                for hook in &entry.plan.ready {
                    self.check_cancelled()?;
                    invoke(*hook, runtime, entry, "就绪").await?;
                }
            }
        }
        self.check_cancelled()
    }

    fn check_cancelled(&self) -> Result<()> {
        ensure!(!self.cancellation.is_cancelled(), "插件启动已取消");
        Ok(())
    }

    pub async fn stop(&self, runtime: &RuntimeState) -> Result<()> {
        self.cancellation.cancel();
        let mut state = self.state.lock().await;
        self.stop_locked(runtime, &mut state).await
    }

    async fn stop_locked(&self, runtime: &RuntimeState, state: &mut LifecycleState) -> Result<()> {
        if let Some(errors) = &state.stopped {
            return cleanup_result(errors);
        }
        self.cancellation.cancel();
        let _requests = self.requests.write().await;
        let mut errors = Vec::new();
        // 所有工作先完成，随后才能释放依赖资源；发布提交期间不会被强制 abort。
        for entry in self.entries.iter().rev() {
            if let Err(error) = entry.tasks.drain().await {
                errors.push(format!("{:?} 后台任务: {error:#}", entry.plan.owner));
            }
        }
        for index in state.entered.iter().rev() {
            let entry = &self.entries[*index];
            for hook in entry.plan.stop.iter().rev() {
                if let Err(error) = invoke(*hook, runtime, entry, "停止").await {
                    errors.push(format!("{error:#}"));
                }
            }
        }
        if !errors.is_empty() {
            eprintln!("插件清理失败: {}", errors.join("; "));
        }
        let result = cleanup_result(&errors);
        state.stopped = Some(errors);
        result
    }
}

fn cleanup_result(errors: &[String]) -> Result<()> {
    ensure!(errors.is_empty(), "插件清理失败: {}", errors.join("; "));
    Ok(())
}

async fn invoke(
    hook: LifecycleHook,
    runtime: &RuntimeState,
    entry: &Entry,
    phase: &str,
) -> Result<()> {
    let result = AssertUnwindSafe(async { hook(runtime.clone(), entry.tasks.clone()).await })
        .catch_unwind()
        .await;
    let result = match result {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("未提供 panic 信息");
            Err(anyhow::anyhow!("插件生命周期 panic: {message}"))
        }
    };
    result.with_context(|| format!("插件 {:?} {phase}失败", entry.plan.owner))
}

struct StartupGuard(Option<RuntimeState>);

impl Drop for StartupGuard {
    fn drop(&mut self) {
        let Some(state) = self.0.take() else { return };
        state.extensions.lifecycle.cancellation.cancel();
        // 调用方丢弃初始化 future 时，当前启动步骤在安全边界结束后仍会清理。
        tokio::spawn(async move {
            let _ = state.extensions.lifecycle.stop(&state).await;
        });
    }
}
