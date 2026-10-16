use std::{
    any::Any,
    collections::{BTreeMap, HashSet},
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::InvocationScope;
use az_plugin_enhancement::{CapabilityCaller, InvocationCapabilities, InvocationControl};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::identity::{IdentityProvider, SessionContext};

/// 宿主能力的副作用约定；幂等写操作必须以业务标识拒绝不同输入的重复请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffect {
    Read,
    IdempotentWrite,
    /// 外部写操作不保证幂等；适配器不得自动重试。
    Write,
}

/// 跨语言能力契约。路径用于业务路由，原生提供者仍按具体 TypeId 校验身份。
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapability {
    pub path: String,
    pub description: String,
    pub effect: CapabilityEffect,
    pub input_schema: schemars::Schema,
    pub output_schema: schemars::Schema,
}

/// 只由运行时创建的调用，身份与插件授权来自真实请求，不读取脚本 context。
pub struct CapabilityRequest {
    pub path: String,
    pub input: Value,
    scope: InvocationScope,
    deadline: std::time::Instant,
}

impl CapabilityRequest {
    /// 整条调用链的截止时间，远端连接及请求均须沿用，不能为子调用重新计时。
    pub fn deadline(&self) -> std::time::Instant {
        self.deadline
    }

    pub fn scope(&self) -> &InvocationScope {
        &self.scope
    }

    /// 用户资源能力必须存在已验证的交互会话；租户后台调用不能借用用户身份。
    pub fn session(&self) -> Result<SessionContext> {
        let context = &self.scope.context;
        let session = context
            .session_id
            .as_deref()
            .context("宿主能力需要用户会话")?;
        ensure!(
            !session.is_empty() && !session.starts_with("service:"),
            "宿主能力需要交互会话"
        );
        Ok(SessionContext {
            session_id: session.into(),
            tenant_id: context.tenant_id.clone().context("宿主能力缺少租户")?,
            user_id: context.user_id.clone().context("宿主能力缺少用户")?,
            account: String::new(),
            display_name: String::new(),
            tenant_label: String::new(),
            permissions: vec![],
        })
    }
}

/// 原生服务或远端协议适配器的扩展点。不得脱离调用另起无人管理的副作用任务；
/// Future 被丢弃时停止等待，非幂等写操作不得自动重试。
#[async_trait::async_trait]
pub trait CapabilityProvider: Any + Send + Sync {
    fn capabilities(&self) -> Vec<RuntimeCapability>;
    /// 租户资源是否已装配；这是可用性检查，不代替运行时的显式能力授权。
    fn available(&self, _path: &str, _tenant: &str) -> bool {
        true
    }
    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value>;
}

/// 从宿主的 Dill 容器解析全部提供者，脚本执行器和协议入口不维护第二套注册表。
pub struct RuntimeCapabilities {
    providers: Vec<Arc<dyn CapabilityProvider>>,
    routes: OnceLock<Result<CapabilityRoutes, String>>,
}

type CapabilityRoutes = BTreeMap<String, (RuntimeCapability, Arc<dyn CapabilityProvider>)>;

#[dill::component(pub)]
#[dill::scope(dill::Singleton)]
impl RuntimeCapabilities {
    /// 只保存 Dill 解析的提供者；能力目录首次校验后固定，不在每次脚本指令重建 schema。
    pub fn new(providers: Vec<Arc<dyn CapabilityProvider>>) -> Self {
        Self {
            providers,
            routes: OnceLock::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.routes()?;
        Ok(())
    }

    fn routes(&self) -> Result<&CapabilityRoutes> {
        self.routes
            .get_or_init(|| self.build_routes().map_err(|error| format!("{error:#}")))
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))
    }

    fn build_routes(&self) -> Result<CapabilityRoutes> {
        let mut types = HashSet::new();
        let mut routes = BTreeMap::new();
        for provider in &self.providers {
            ensure!(
                types.insert(provider.as_ref().type_id()),
                "宿主能力提供者类型被重复注册"
            );
            let capabilities = provider.capabilities();
            ensure!(!capabilities.is_empty(), "宿主能力提供者未声明任何能力");
            for capability in capabilities {
                az_plugin_contract::validate_enhancement_path(&capability.path)?;
                ensure!(
                    !routes.contains_key(&capability.path),
                    "多个提供者声明了同一宿主能力路径"
                );
                routes.insert(capability.path.clone(), (capability, provider.clone()));
            }
        }
        Ok(routes)
    }

    /// 目录、候选激活与调用都只使用该租户已装配的能力。
    pub fn catalog(&self, tenant: &str) -> Result<Vec<RuntimeCapability>> {
        Ok(self
            .routes()?
            .values()
            .filter(|(capability, provider)| provider.available(&capability.path, tenant))
            .map(|(capability, _)| capability.clone())
            .collect())
    }

    fn provider(&self, path: &str) -> Result<Arc<dyn CapabilityProvider>> {
        let (_, provider) = self.routes()?.get(path).context("宿主能力提供者未装配")?;
        Ok(provider.clone())
    }
}

pub(in crate::runtime::server) struct CapabilityEnvironment {
    pub services: Weak<super::super::plugins::RuntimeExtensions>,
    pub identity: Arc<dyn IdentityProvider>,
}

impl CapabilityEnvironment {
    pub(super) fn runtime(&self) -> Result<Arc<RuntimeCapabilities>> {
        let services = self.services.upgrade().context("宿主能力环境已释放")?;
        Ok(services.services.get_one::<RuntimeCapabilities>()?)
    }

    pub(super) fn bind(&self, scope: &InvocationScope) -> Result<InvocationCapabilities> {
        Ok(InvocationCapabilities {
            grants: scope.grants.runtime.clone(),
            caller: Some(Arc::new(Caller {
                runtime: self.runtime()?,
                identity: self.identity.clone(),
                scope: scope.clone(),
                handle: tokio::runtime::Handle::try_current().context("宿主能力缺少异步运行时")?,
            })),
        })
    }
}

struct Caller {
    runtime: Arc<RuntimeCapabilities>,
    identity: Arc<dyn IdentityProvider>,
    scope: InvocationScope,
    handle: tokio::runtime::Handle,
}

impl CapabilityCaller for Caller {
    fn available(&self, path: &str) -> bool {
        self.runtime.provider(path).is_ok_and(|provider| {
            self.scope
                .context
                .tenant_id
                .as_deref()
                .is_some_and(|tenant| provider.available(path, tenant))
        })
    }

    fn call(&self, path: &str, input: &Value, control: &InvocationControl) -> Result<Value> {
        control.check()?;
        let provider = self.runtime.provider(path)?;
        let request = CapabilityRequest {
            path: path.into(),
            input: input.clone(),
            scope: self.scope.clone(),
            deadline: control.deadline(),
        };
        // 解释器位于 spawn_blocking 中；只在该线程等待，异步 I/O 仍由宿主运行时驱动。
        self.handle.block_on(async {
            let operation = async {
                let context = &self.scope.context;
                let tenant = context.tenant_id.as_deref().context("宿主能力缺少租户")?;
                ensure!(provider.available(path, tenant), "租户未装配该宿主能力");
                match (&context.user_id, &context.session_id) {
                    (Some(user), Some(session)) => {
                        ensure!(
                            self.identity.member_active(tenant, user).await?
                                && self.identity.session_active(session, tenant, user).await?,
                            "宿主能力调用者已撤权"
                        );
                    }
                    (None, None) => {}
                    _ => anyhow::bail!("宿主能力的用户会话不完整"),
                }
                control.check()?;
                provider.invoke(&request).await
            };
            tokio::pin!(operation);
            let mut checks = tokio::time::interval(Duration::from_millis(5));
            checks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let deadline = tokio::time::sleep_until(control.deadline().into());
            tokio::pin!(deadline);
            loop {
                control.check()?;
                tokio::select! {
                    output = &mut operation => return output,
                    _ = &mut deadline => anyhow::bail!("宿主能力等待超时"),
                    _ = checks.tick() => {},
                }
            }
        })
    }
}
