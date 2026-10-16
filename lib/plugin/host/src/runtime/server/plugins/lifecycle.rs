//! 插件启动、失败补偿与关闭；异步清理由独立任务持有，调用方取消等待不会打断清理。

use std::{any::TypeId, collections::HashSet, panic::AssertUnwindSafe, sync::Arc};

use anyhow::{Context, Result, ensure};
use futures_util::FutureExt;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use super::{LifecycleHook, PluginTasks, RuntimeState};

#[derive(Clone)]
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
    entries: Vec<Arc<Entry>>,
    inherited: HashSet<TypeId>,
    pub cancellation: CancellationToken,
    pub requests: Arc<RwLock<()>>,
    state: Mutex<LifecycleState>,
}

#[derive(Default)]
struct LifecycleState {
    started: bool,
    entered: Vec<usize>,
    stopped: Option<Vec<String>>,
    retired: HashSet<TypeId>,
}

impl Lifecycle {
    pub fn compose(
        plans: Vec<HookPlan>,
        previous: Option<&Self>,
        changed: &HashSet<TypeId>,
    ) -> Self {
        let mut inherited = HashSet::new();
        let entries = plans
            .into_iter()
            .map(|plan| {
                if !changed.contains(&plan.owner)
                    && let Some(entry) = previous.and_then(|old| {
                        old.entries
                            .iter()
                            .find(|entry| entry.plan.owner == plan.owner)
                    })
                {
                    inherited.insert(plan.owner);
                    return entry.clone();
                }
                Arc::new(Entry {
                    plan,
                    tasks: Arc::new(PluginTasks::new(CancellationToken::new())),
                })
            })
            .collect();
        Self {
            entries,
            inherited,
            cancellation: CancellationToken::new(),
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
            return match self
                .stop_locked(runtime, &mut state, Some(&self.fresh()))
                .await
            {
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
            if self.inherited.contains(&entry.plan.owner) {
                continue;
            }
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
                if self.inherited.contains(&entry.plan.owner) {
                    continue;
                }
                for hook in &entry.plan.start {
                    self.check_cancelled()?;
                    invoke(*hook, runtime, entry, "启动").await?;
                }
            }
        }
        if run_hooks {
            for entry in &self.entries {
                if self.inherited.contains(&entry.plan.owner) {
                    continue;
                }
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

    pub(super) fn fresh(&self) -> HashSet<TypeId> {
        self.entries
            .iter()
            .filter(|entry| !self.inherited.contains(&entry.plan.owner))
            .map(|entry| entry.plan.owner)
            .collect()
    }

    pub async fn stop(&self, runtime: &RuntimeState) -> Result<()> {
        self.stop_selected(runtime, None).await
    }

    pub(super) async fn stop_selected(
        &self,
        runtime: &RuntimeState,
        selected: Option<&HashSet<TypeId>>,
    ) -> Result<()> {
        self.cancellation.cancel();
        for entry in &self.entries {
            if selected.is_none_or(|owners| owners.contains(&entry.plan.owner)) {
                entry.tasks.cancellation().cancel();
            }
        }
        let mut state = self.state.lock().await;
        self.stop_locked(runtime, &mut state, selected).await
    }

    pub(super) async fn cleanup_succeeded(&self) -> bool {
        self.state
            .lock()
            .await
            .stopped
            .as_ref()
            .is_some_and(Vec::is_empty)
    }

    async fn stop_locked(
        &self,
        runtime: &RuntimeState,
        state: &mut LifecycleState,
        selected: Option<&HashSet<TypeId>>,
    ) -> Result<()> {
        self.cancellation.cancel();
        let retired = state.retired.clone();
        let selected_entry = |entry: &Arc<Entry>| {
            !retired.contains(&entry.plan.owner)
                && selected.is_none_or(|owners| owners.contains(&entry.plan.owner))
        };
        // 请求代数与后台任务分别取消，保留插件的任务不能被旧代数的退役连带停止。
        for entry in self.entries.iter().filter(|entry| selected_entry(entry)) {
            entry.tasks.cancellation().cancel();
        }
        let _requests = self.requests.write().await;
        let mut errors = state.stopped.clone().unwrap_or_default();
        // 所有工作先完成，随后才能释放依赖资源；发布提交期间不会被强制 abort。
        for entry in self
            .entries
            .iter()
            .rev()
            .filter(|entry| selected_entry(entry))
        {
            if let Err(error) = entry.tasks.drain().await {
                errors.push(format!("{:?} 后台任务: {error:#}", entry.plan.owner));
            }
        }
        for index in state.entered.iter().rev() {
            let entry = &self.entries[*index];
            if !selected_entry(entry) {
                continue;
            }
            for hook in entry.plan.stop.iter().rev() {
                if let Err(error) = invoke(*hook, runtime, entry, "停止").await {
                    errors.push(format!("{error:#}"));
                }
            }
        }
        if !errors.is_empty() {
            eprintln!("插件清理失败: {}", errors.join("; "));
        }
        state.retired.extend(
            self.entries
                .iter()
                .filter(|entry| selected_entry(entry))
                .map(|entry| entry.plan.owner),
        );
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
            let lifecycle = &state.extensions.lifecycle;
            let _ = lifecycle
                .stop_selected(&state, Some(&lifecycle.fresh()))
                .await;
        });
    }
}
