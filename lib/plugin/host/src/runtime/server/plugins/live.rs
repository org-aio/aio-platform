//! 在线组合的唯一所有者；请求持有不可变代数，提交及恢复不受调用方取消影响。

use std::{
    any::TypeId,
    collections::HashSet,
    panic::AssertUnwindSafe,
    sync::{Arc, RwLock},
};

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use az_plugin_core::{DynPlugin, Plugin};
use tokio::sync::Mutex;
use tower::ServiceExt;

use super::{RuntimeAssembly, RuntimeState, composition};
use crate::configuration::HostConfig;

#[derive(Clone)]
struct Generation {
    state: RuntimeState,
    router: Router,
}

struct HostState {
    generation: RwLock<Generation>,
    operation: Arc<Mutex<()>>,
}

/// 管理同一宿主内的原生插件；不提供跨进程 TypeId 或任意管理 HTTP 入口。
#[derive(Clone)]
pub struct RuntimeHost(Arc<HostState>);

impl RuntimeHost {
    pub async fn initialize(
        config: HostConfig,
        plugins: Vec<DynPlugin<RuntimeAssembly>>,
    ) -> Result<Self> {
        let state = RuntimeState::initialize(config, plugins).await?;
        let generation = match Generation::new(state.clone()) {
            Ok(generation) => generation,
            Err(error) => {
                return match state.shutdown().await {
                    Ok(()) => Err(error),
                    Err(cleanup) => {
                        Err(error.context(format!("路由装配失败且清理失败: {cleanup:#}")))
                    }
                };
            }
        };
        Ok(Self(Arc::new(HostState {
            generation: RwLock::new(generation),
            operation: Arc::new(Mutex::new(())),
        })))
    }

    /// 返回长期使用的分发入口；替换插件后无需重新绑定监听器。
    /// 集成其他应用路由时，把这个服务挂在独立路由空间，避免覆盖应用的 fallback。
    pub fn router(&self) -> Router {
        let host = self.clone();
        Router::new().fallback_service(tower::service_fn(move |request| {
            let host = host.clone();
            async move { Ok::<_, std::convert::Infallible>(host.dispatch(request).await) }
        }))
    }

    async fn dispatch(&self, request: Request) -> Response {
        let router = match self.current() {
            Ok(generation) => generation.router,
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }

    /// 调用方保留未变插件的 Arc；只替换指定配方和它们的传递依赖。
    /// build 必须只声明服务，资源创建放入初始化钩子，每次重建需提供全新资源。
    pub async fn reconfigure(&self, plugins: Vec<DynPlugin<RuntimeAssembly>>) -> Result<()> {
        let guard = self.0.operation.clone().lock_owned().await;
        self.change(plugins, HashSet::new(), guard).await
    }

    /// 按具体插件类型重启；依赖该插件的服务一起重建，无关服务保留原单例及任务。
    pub async fn restart<P: Plugin<RuntimeAssembly>>(&self) -> Result<()> {
        let guard = self.0.operation.clone().lock_owned().await;
        let old = self.current()?;
        let owner = TypeId::of::<P>();
        ensure!(
            old.state
                .extensions
                .contributions
                .iter()
                .any(|entry| entry.owner() == owner),
            "不能重启未安装的插件"
        );
        let plugins = old
            .state
            .extensions
            .contributions
            .iter()
            .map(|entry| entry.plugin.clone())
            .collect();
        self.change(plugins, HashSet::from([owner]), guard).await
    }

    async fn change(
        &self,
        plugins: Vec<DynPlugin<RuntimeAssembly>>,
        mut changed: HashSet<TypeId>,
        guard: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<()> {
        let old = self.current()?;
        ensure!(
            !old.state.extensions.lifecycle.cancellation.is_cancelled(),
            "宿主已停止或恢复失败"
        );
        super::validate_selection(&old.state.config, &plugins)?;
        let previous = &old.state.extensions.contributions;
        for entry in previous {
            if !plugins
                .iter()
                .any(|plugin| Arc::ptr_eq(plugin, &entry.plugin))
            {
                changed.insert(entry.owner());
            }
        }
        for plugin in &plugins {
            if !previous
                .iter()
                .any(|entry| Arc::ptr_eq(plugin, &entry.plugin))
            {
                changed.insert(plugin.as_ref().type_id());
            }
        }
        let mut next = composition::build(
            old.state.config.clone(),
            plugins.clone(),
            previous,
            &changed,
        )
        .await?;
        // 新配方可能改变接口依赖；继续扩展闭包，直到所有受影响贡献都已重建。
        loop {
            let affected = composition::affected(previous, &next, changed.clone());
            let additional = affected
                .difference(&changed)
                .copied()
                .collect::<HashSet<_>>();
            changed = affected;
            if additional.is_empty() {
                break;
            }
            next = composition::build(
                old.state.config.clone(),
                plugins.clone(),
                &next,
                &additional,
            )
            .await?;
        }
        if changed.is_empty() {
            return Ok(());
        }
        let candidate = composition::assemble(next, Some(&old.state.extensions), &changed)?;
        let previous_plugins = previous.iter().map(|entry| entry.plugin.clone()).collect();
        let recovery = composition::build(
            old.state.config.clone(),
            previous_plugins,
            previous,
            &changed,
        )
        .await?;
        let recovery = composition::assemble(recovery, Some(&old.state.extensions), &changed)?;
        let candidate = old.state.with_extensions(candidate);
        let recovery = old.state.with_extensions(recovery);
        let host = self.clone();
        // 进入停用阶段后由独立任务持有操作锁；丢弃调用 future 不会留下半切换宿主。
        tokio::spawn(async move {
            let _guard = guard;
            host.commit(old, candidate, recovery, changed).await
        })
        .await
        .context("插件在线变更任务未正常结束")?
    }

    async fn commit(
        &self,
        old: Generation,
        candidate: RuntimeState,
        recovery: RuntimeState,
        changed: HashSet<TypeId>,
    ) -> Result<()> {
        let lifecycle = &old.state.extensions.lifecycle;
        if let Err(error) = lifecycle.stop_selected(&old.state, Some(&changed)).await {
            let cleanup = lifecycle.stop(&old.state).await;
            return Err(error.context(format!(
                "停止旧插件失败，宿主已停止接收请求；剩余清理: {cleanup:?}"
            )));
        }
        let run_hooks = old.state.config.development.is_none();
        match Generation::start(candidate.clone(), run_hooks).await {
            Ok(generation) => self.publish(generation),
            Err(error) => {
                if !candidate.extensions.lifecycle.cleanup_succeeded().await {
                    let cleanup = lifecycle.stop(&old.state).await;
                    return Err(error.context(format!(
                        "新插件清理失败，停止恢复以防资源重叠；剩余清理: {cleanup:?}"
                    )));
                }
                match Generation::start(recovery, run_hooks).await {
                    Ok(generation) => {
                        self.publish(generation)?;
                        Err(error.context("新插件启动失败，已用旧配方重建并恢复服务"))
                    }
                    Err(recovery_error) => {
                        let cleanup = lifecycle.stop(&old.state).await;
                        Err(error.context(format!("恢复也失败，宿主已停止接收请求: {recovery_error:#}; 剩余清理: {cleanup:?}")))
                    }
                }
            }
        }
    }

    fn current(&self) -> Result<Generation> {
        self.0
            .generation
            .read()
            .map(|generation| generation.clone())
            .map_err(|_| anyhow::anyhow!("运行时代数锁已损坏"))
    }

    fn publish(&self, generation: Generation) -> Result<()> {
        *self
            .0
            .generation
            .write()
            .map_err(|_| anyhow::anyhow!("运行时代数锁已损坏"))? = generation;
        Ok(())
    }

    /// 停止当前代数；与在线变更串行，并保留原有的协作取消及反序清理语义。
    pub async fn shutdown(&self) -> Result<()> {
        let host = self.clone();
        tokio::spawn(async move {
            let _guard = host.0.operation.lock().await;
            host.current()?.state.shutdown().await
        })
        .await
        .context("在线宿主停止任务未正常结束")?
    }
}

impl Generation {
    async fn start(state: RuntimeState, run_hooks: bool) -> Result<Self> {
        let lifecycle = &state.extensions.lifecycle;
        state.extensions.start(&state, run_hooks).await?;
        match Self::new(state.clone()) {
            Ok(generation) => Ok(generation),
            Err(error) => {
                let cleanup = lifecycle
                    .stop_selected(&state, Some(&lifecycle.fresh()))
                    .await;
                Err(error.context(format!("路由装配失败，已清理新插件: {cleanup:?}")))
            }
        }
    }

    fn new(state: RuntimeState) -> Result<Self> {
        let router =
            std::panic::catch_unwind(AssertUnwindSafe(|| super::super::router(state.clone())))
                .map_err(|_| anyhow::anyhow!("插件路由冲突或路由工厂 panic"))?;
        Ok(Self { state, router })
    }
}
