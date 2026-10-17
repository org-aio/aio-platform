//! 按插件归属保存装配配方；依赖信息只用于变更规划，服务解析始终由 Dill 完成。

use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    panic::AssertUnwindSafe,
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use az_plugin_core::{App, DynPlugin};
use dill::{Catalog, CatalogBuilder, CatalogWeakRef};
use futures_util::FutureExt;

use super::{RuntimeAssembly, RuntimeExtensions, ServiceBindings, lifecycle::Lifecycle};
use crate::configuration::HostConfig;

#[derive(Clone)]
pub(super) struct Contribution {
    pub plugin: DynPlugin<RuntimeAssembly>,
    pub assembly: RuntimeAssembly,
}

impl Contribution {
    pub fn owner(&self) -> TypeId {
        self.plugin.as_ref().type_id()
    }

    fn observes_catalog(&self) -> bool {
        self.assembly.catalog_sensitive
            || self
                .assembly
                .services
                .dependencies
                .contains(&TypeId::of::<Catalog>())
            || self
                .assembly
                .services
                .dependencies
                .contains(&TypeId::of::<CatalogWeakRef>())
    }

    fn depends_on(&self, provider: &Self) -> bool {
        self.plugin.dependencies().contains(&provider.owner())
            || !self
                .assembly
                .service_dependencies
                .is_disjoint(&provider.assembly.services.interfaces)
            || !self
                .assembly
                .services
                .dependencies
                .is_disjoint(&provider.assembly.services.interfaces)
    }
}

impl RuntimeAssembly {
    fn new(config: Arc<HostConfig>) -> Self {
        Self {
            services: ServiceBindings::new(),
            config,
            routes: Vec::new(),
            requirements: Vec::new(),
            lifecycle: Vec::new(),
            service_dependencies: HashSet::new(),
            catalog_sensitive: false,
        }
    }
}

pub(super) async fn build(
    config: Arc<HostConfig>,
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
    previous: &[Contribution],
    rebuild: &HashSet<TypeId>,
) -> Result<Vec<Contribution>> {
    let app = App::new(RuntimeAssembly::new(config.clone()));
    let plugins = app.plan_plugins(plugins, &HashMap::new())?;
    let mut contributions = Vec::new();
    for plugin in plugins {
        let owner = plugin.as_ref().type_id();
        let existing = previous
            .iter()
            .find(|old| !rebuild.contains(&owner) && Arc::ptr_eq(&old.plugin, &plugin));
        if let Some(existing) = existing {
            contributions.push(existing.clone());
            continue;
        }
        let mut assembly = RuntimeAssembly::new(config.clone());
        AssertUnwindSafe(async { plugin.build(&mut assembly).await })
            .catch_unwind()
            .await
            .map_err(|_| anyhow::anyhow!("插件 {owner:?} 装配时 panic"))?
            .with_context(|| format!("构建插件失败: {owner:?}"))?;
        ensure!(
            assembly.lifecycle.iter().all(|plan| plan.owner == owner),
            "插件只能声明自己的生命周期"
        );
        contributions.push(Contribution { plugin, assembly });
    }
    // 必需或可选服务均可指向异步资源，存在的提供者必须先于使用方初始化。
    // 聚合服务的完整 Dill 注入图只决定重建范围，不把延迟聚合误作初始化环。
    let additional = contributions
        .iter()
        .map(|consumer| {
            let providers = contributions
                .iter()
                .filter(|provider| {
                    provider.owner() != consumer.owner()
                        && !consumer
                            .assembly
                            .service_dependencies
                            .is_disjoint(&provider.assembly.services.interfaces)
                })
                .map(Contribution::owner)
                .collect();
            (consumer.owner(), providers)
        })
        .collect();
    let plugins = contributions
        .iter()
        .map(|entry| entry.plugin.clone())
        .collect();
    let ordered = app.plan_plugins(plugins, &additional)?;
    let positions = ordered
        .iter()
        .enumerate()
        .map(|(index, plugin)| (plugin.as_ref().type_id(), index))
        .collect::<HashMap<_, _>>();
    contributions.sort_by_key(|entry| positions[&entry.owner()]);
    Ok(contributions)
}

/// 显式插件依赖、接口使用和 Dill 注入共同决定重建范围，包含旧图与新图的传递依赖。
pub(super) fn affected(
    old: &[Contribution],
    next: &[Contribution],
    mut changed: HashSet<TypeId>,
) -> HashSet<TypeId> {
    let all = old.iter().chain(next).collect::<Vec<_>>();
    if changed.is_empty() {
        return changed;
    }
    loop {
        let before = changed.len();
        for consumer in &all {
            if consumer.observes_catalog()
                || all.iter().any(|provider| {
                    changed.contains(&provider.owner()) && consumer.depends_on(provider)
                })
            {
                changed.insert(consumer.owner());
            }
        }
        if changed.len() == before {
            return changed;
        }
    }
}

pub(super) fn assemble(
    contributions: Vec<Contribution>,
    previous: Option<&RuntimeExtensions>,
    changed: &HashSet<TypeId>,
) -> Result<Arc<RuntimeExtensions>> {
    let mut catalog = CatalogBuilder::new();
    let mut implementations = HashSet::new();
    let mut routes = Vec::new();
    let mut requirements = Vec::new();
    let mut plans = Vec::new();
    for contribution in &contributions {
        let assembly = &contribution.assembly;
        for implementation in &assembly.services.implementations {
            ensure!(
                implementations.insert(*implementation),
                "多个插件重复声明服务实现: {implementation:?}"
            );
        }
        assembly.services.register(&mut catalog)?;
        routes.extend_from_slice(&assembly.routes);
        requirements.extend_from_slice(&assembly.requirements);
        plans.extend(assembly.lifecycle.iter().cloned());
    }
    catalog.validate().context("校验宿主能力依赖失败")?;
    let services = catalog.build();
    super::validate_security_provider::<dyn crate::identity::IdentityProvider>(&services)?;
    super::validate_security_provider::<dyn crate::authorization::AuthorizationProvider>(
        &services,
    )?;
    super::validate_security_provider::<dyn crate::metering::MeteringProvider>(&services)?;
    for validate in &requirements {
        validate(&services, false)?;
    }
    Ok(Arc::new(RuntimeExtensions {
        services,
        routes,
        lifecycle: Lifecycle::compose(plans, previous.map(|old| &old.lifecycle), changed),
        requirements,
        contributions,
    }))
}
