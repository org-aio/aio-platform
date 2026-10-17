use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::InvocationScope;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{CapabilityRequest, RuntimeCapabilities, catalog::CapabilityRoutes};
use crate::runtime::server::plugins::PluginTasks;
use crate::{identity::IdentityProvider, runtime::server::RuntimeState};

/// 生命周期由能力插件持有；停止和重建会取消已发出的调用快照。
pub(in crate::runtime::server) struct CapabilityLifetime(pub Arc<PluginTasks>);

/// 一条能力调用链固定身份、授权与提供者快照；克隆共享预算、截止时间和取消状态。
#[derive(Clone)]
pub struct CapabilityInvocation(Arc<Invocation>);

struct Invocation {
    routes: Arc<CapabilityRoutes>,
    identity: Arc<dyn IdentityProvider>,
    scope: InvocationScope,
    deadline: Instant,
    cancellation: CancellationToken,
    tasks: Arc<PluginTasks>,
    calls: AtomicUsize,
    bytes: AtomicUsize,
}

impl RuntimeState {
    /// 原生插件以宿主已验证的调用范围创建执行链，不需要装配任何脚本执行器。
    /// 入口负责建立真实身份与授权；这里继续核验显式能力、租户可用性和会话存活。
    pub fn capability_invocation(
        &self,
        scope: &InvocationScope,
        deadline: Option<Instant>,
    ) -> Result<CapabilityInvocation> {
        self.service::<RuntimeCapabilities>()?.bind(
            scope,
            self.service::<dyn IdentityProvider>()?,
            self.service::<CapabilityLifetime>()?.0.clone(),
            deadline,
        )
    }
}

impl RuntimeCapabilities {
    pub(in crate::runtime::server) fn bind(
        &self,
        scope: &InvocationScope,
        identity: Arc<dyn IdentityProvider>,
        tasks: Arc<PluginTasks>,
        deadline: Option<Instant>,
    ) -> Result<CapabilityInvocation> {
        let maximum = Instant::now() + Duration::from_secs(30);
        let invocation = CapabilityInvocation(Arc::new(Invocation {
            routes: self.routes()?,
            identity,
            scope: scope.clone(),
            deadline: deadline.unwrap_or(maximum).min(maximum),
            cancellation: tasks.cancellation().child_token(),
            tasks,
            calls: AtomicUsize::new(64),
            bytes: AtomicUsize::new(1024 * 1024),
        }));
        invocation.check()?;
        Ok(invocation)
    }
}

impl CapabilityInvocation {
    pub fn cancel(&self) {
        self.0.cancellation.cancel();
    }

    pub fn deadline(&self) -> Instant {
        self.0.deadline
    }

    pub fn available(&self, path: &str) -> bool {
        self.check().is_ok()
            && self
                .0
                .scope
                .grants
                .runtime
                .iter()
                .any(|grant| grant == path)
            && self.0.routes.get(path).is_some_and(|route| {
                self.0
                    .scope
                    .context
                    .tenant_id
                    .as_deref()
                    .is_some_and(|tenant| route.provider.available(path, tenant))
            })
    }

    /// 异步执行原生或远端能力；取消 Future 会丢弃正在等待的上游调用，不自动重试。
    pub async fn invoke(&self, path: &str, input: &Value) -> Result<Value> {
        self.execute(path, input, self.deadline(), || Ok(())).await
    }

    pub(in crate::runtime::server) async fn execute(
        &self,
        path: &str,
        input: &Value,
        deadline: Instant,
        check: impl Fn() -> Result<()>,
    ) -> Result<Value> {
        let _permit = self.0.tasks.enter().await?;
        self.check()?;
        check()?;
        self.consume(&self.0.calls, 1)?;
        ensure!(
            self.0
                .scope
                .grants
                .runtime
                .iter()
                .any(|grant| grant == path),
            "调用者未获宿主能力: {path}"
        );
        let route = self.0.routes.get(path).context("宿主能力提供者未装配")?;
        self.check_value(input)?;
        route.input.check(input, "宿主能力输入")?;
        let deadline = deadline.min(self.deadline());
        let request = CapabilityRequest {
            path: path.into(),
            input: input.clone(),
            scope: self.0.scope.clone(),
            deadline,
        };
        let operation = async {
            let context = &self.0.scope.context;
            let tenant = context.tenant_id.as_deref().context("宿主能力缺少租户")?;
            ensure!(!tenant.is_empty(), "宿主能力缺少租户");
            ensure!(
                route.provider.available(path, tenant),
                "租户未装配该宿主能力"
            );
            match (&context.user_id, &context.session_id) {
                (Some(user), Some(session)) => ensure!(
                    self.0.identity.member_active(tenant, user).await?
                        && self
                            .0
                            .identity
                            .session_active(session, tenant, user)
                            .await?,
                    "宿主能力调用者已撤权"
                ),
                (None, None) => {}
                _ => anyhow::bail!("宿主能力的用户会话不完整"),
            }
            self.check()?;
            check()?;
            ensure!(Instant::now() < deadline, "宿主能力等待超时");
            route.provider.invoke(&request).await
        };
        tokio::pin!(operation);
        let mut checks = tokio::time::interval(Duration::from_millis(5));
        checks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let timeout = tokio::time::sleep_until(deadline.into());
        tokio::pin!(timeout);
        let output = loop {
            self.check()?;
            check()?;
            ensure!(Instant::now() < deadline, "宿主能力等待超时");
            tokio::select! {
                output = &mut operation => break output?,
                _ = &mut timeout => anyhow::bail!("宿主能力等待超时"),
                _ = self.0.cancellation.cancelled() => anyhow::bail!("宿主能力调用已取消"),
                _ = checks.tick() => {},
            }
        };
        self.check()?;
        check()?;
        self.check_value(&output)?;
        route.output.check(&output, "宿主能力输出")?;
        self.check()?;
        check()?;
        ensure!(Instant::now() < deadline, "宿主能力等待超时");
        Ok(output)
    }

    fn check(&self) -> Result<()> {
        ensure!(!self.0.cancellation.is_cancelled(), "宿主能力调用已取消");
        ensure!(Instant::now() < self.deadline(), "宿主能力等待超时");
        Ok(())
    }

    fn check_value(&self, value: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(value)?.len();
        ensure!(
            bytes <= az_plugin_enhancement::MAX_VALUE_BYTES,
            "宿主能力值超过配额"
        );
        self.consume(&self.0.bytes, bytes)
    }

    fn consume(&self, remaining: &AtomicUsize, amount: usize) -> Result<()> {
        if remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(amount)
            })
            .is_err()
        {
            self.cancel();
            anyhow::bail!("宿主能力调用链预算已耗尽");
        }
        Ok(())
    }
}
