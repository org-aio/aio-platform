use std::{
    any::TypeId,
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Result, ensure};
use az_plugin_contract::ValueSchema;

use super::{CapabilityProvider, RuntimeCapability};

/// 从宿主的 Dill 容器解析全部提供者，脚本执行器和协议入口不维护第二套注册表。
pub struct RuntimeCapabilities {
    providers: Vec<Arc<dyn CapabilityProvider>>,
    registry: OnceLock<Result<Mutex<Registry>, String>>,
}

#[derive(Clone)]
pub(super) struct CapabilityRoute {
    owner: TypeId,
    pub capability: RuntimeCapability,
    pub provider: Arc<dyn CapabilityProvider>,
    pub input: Arc<ValueSchema>,
    pub output: Arc<ValueSchema>,
}

pub(super) type CapabilityRoutes = BTreeMap<String, CapabilityRoute>;

struct Registry {
    routes: Arc<CapabilityRoutes>,
    dynamic: HashSet<TypeId>,
    // 删除后仍保留契约，防止旧脚本在同一路径重新出现时获得不同副作用语义。
    contracts: BTreeMap<String, (TypeId, RuntimeCapability)>,
}

#[dill::component(pub)]
#[dill::scope(dill::Singleton)]
impl RuntimeCapabilities {
    /// 只保存 Dill 解析的提供者；目录与已编译 schema 按快照共享，不在脚本指令中重建。
    pub fn new(providers: Vec<Arc<dyn CapabilityProvider>>) -> Self {
        Self {
            providers,
            registry: OnceLock::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.registry()?;
        Ok(())
    }

    fn registry(&self) -> Result<&Mutex<Registry>> {
        self.registry
            .get_or_init(|| {
                self.build_registry()
                    .map(Mutex::new)
                    .map_err(|error| format!("{error:#}"))
            })
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))
    }

    fn build_registry(&self) -> Result<Registry> {
        let mut types = HashSet::new();
        let mut routes = BTreeMap::new();
        let mut contracts = BTreeMap::new();
        let mut dynamic = HashSet::new();
        for provider in &self.providers {
            let owner = provider.as_ref().type_id();
            ensure!(types.insert(owner), "宿主能力提供者类型被重复注册");
            let snapshot = provider.snapshot();
            if snapshot.is_some() {
                dynamic.insert(owner);
            }
            let snapshot = snapshot.unwrap_or_else(|| provider.clone());
            ensure!(
                dynamic.contains(&owner) || !snapshot.capabilities().is_empty(),
                "宿主能力提供者未声明任何能力"
            );
            add_routes(&mut routes, &mut contracts, owner, snapshot)?;
        }
        Ok(Registry {
            routes: Arc::new(routes),
            dynamic,
            contracts,
        })
    }

    pub(super) fn routes(&self) -> Result<Arc<CapabilityRoutes>> {
        let registry = self
            .registry()?
            .lock()
            .map_err(|_| anyhow::anyhow!("能力目录锁已损坏"))?;
        Ok(registry.routes.clone())
    }

    /// 原生动态提供者在候选完全准备好后发布；先校验全局路径及历史契约，再同步提交。
    /// commit 只更新提供者的内存快照，不得执行 I/O 或递归访问目录；返回错误前不得修改状态。
    /// 校验或提交失败保留整个旧目录。
    pub fn publish<P: CapabilityProvider>(
        &self,
        snapshot: Arc<dyn CapabilityProvider>,
        commit: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let owner = TypeId::of::<P>();
        let mut registry = self
            .registry()?
            .lock()
            .map_err(|_| anyhow::anyhow!("能力目录锁已损坏"))?;
        ensure!(
            registry.dynamic.contains(&owner),
            "提供者未通过 Dill 声明动态能力"
        );
        let mut routes = registry.routes.as_ref().clone();
        routes.retain(|_, route| route.owner != owner);
        let mut contracts = registry.contracts.clone();
        add_routes(&mut routes, &mut contracts, owner, snapshot)?;
        commit()?;
        registry.routes = Arc::new(routes);
        registry.contracts = contracts;
        Ok(())
    }

    /// 目录、候选激活与调用都只使用该租户已装配的能力。
    pub fn catalog(&self, tenant: &str) -> Result<Vec<RuntimeCapability>> {
        Ok(self
            .routes()?
            .values()
            .filter(|route| route.provider.available(&route.capability.path, tenant))
            .map(|route| route.capability.clone())
            .collect())
    }
}

fn add_routes(
    routes: &mut CapabilityRoutes,
    contracts: &mut BTreeMap<String, (TypeId, RuntimeCapability)>,
    owner: TypeId,
    provider: Arc<dyn CapabilityProvider>,
) -> Result<()> {
    for capability in provider.capabilities() {
        az_plugin_contract::validate_enhancement_path(&capability.path)?;
        ensure!(
            !routes.contains_key(&capability.path),
            "多个提供者声明了同一宿主能力路径"
        );
        if let Some((previous_owner, previous)) = contracts.get(&capability.path) {
            ensure!(
                *previous_owner == owner && previous == &capability,
                "已有能力路径的提供者和契约不可变，请使用新版本路径"
            );
        }
        contracts.insert(capability.path.clone(), (owner, capability.clone()));
        routes.insert(
            capability.path.clone(),
            CapabilityRoute {
                owner,
                input: Arc::new(ValueSchema::compile(Some(&capability.input_schema))?),
                output: Arc::new(ValueSchema::compile(Some(&capability.output_schema))?),
                capability,
                provider: provider.clone(),
            },
        );
    }
    ensure!(contracts.len() <= 4096, "进程内能力契约历史超过 4096 项");
    Ok(())
}
