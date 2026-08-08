use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use az_plugin_core::plugin::{BackendApiContribution, BackendPageContribution, ContributionSet};

use crate::{
    DefinitionState, EndpointImplementationDefinition, PageDefinition, PageEndpointDefinition,
    PageRendererDefinition, ProgramDefinition, RestMethod, RouteDefinition, SymbolId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeContractCatalog {
    pages: Vec<NativePageContract>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativePageContract {
    plugin_id: String,
    page: BackendPageContribution,
    apis: Vec<BackendApiContribution>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeContractReconcileReport {
    pub changed: bool,
    pub pages_created: usize,
    pub routes_created: usize,
    pub endpoints_created: usize,
    pub endpoints_updated: usize,
    pub endpoints_removed: usize,
}

impl NativeContractCatalog {
    pub fn from_contributions<'a>(
        contributions: impl IntoIterator<Item = (&'a str, &'a ContributionSet)>,
    ) -> Result<Self> {
        let mut pages = Vec::new();
        let mut routes = BTreeSet::new();
        for (plugin_id, contribution) in contributions {
            if contribution.backend_apis.is_empty() {
                continue;
            }
            let page = contribution.backend_page.clone().with_context(|| {
                format!("插件 {plugin_id} 声明了后端接口但没有 Studio 页面归属")
            })?;
            for api in &contribution.backend_apis {
                let method = parse_rest_method(&api.method)?;
                if !routes.insert((method, api.path.clone())) {
                    bail!("原生接口重复: {} {}", method.as_str(), api.path);
                }
            }
            pages.push(NativePageContract {
                plugin_id: plugin_id.to_owned(),
                page,
                apis: contribution.backend_apis.clone(),
            });
        }
        pages.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
        Ok(Self { pages })
    }

    pub fn reconcile(
        &self,
        definition: &mut ProgramDefinition,
    ) -> Result<NativeContractReconcileReport> {
        let original = definition.clone();
        let expected = self.expected_endpoints()?;
        let mut report = NativeContractReconcileReport::default();

        for page in &mut definition.pages {
            let before = page.endpoints.len();
            page.endpoints
                .retain(|endpoint| match &endpoint.implementation {
                    EndpointImplementationDefinition::Native { plugin_id } => expected.contains(&(
                        page.name.clone(),
                        plugin_id.clone(),
                        endpoint.method,
                        endpoint.path.clone(),
                    )),
                    EndpointImplementationDefinition::Convention => true,
                });
            report.endpoints_removed += before.saturating_sub(page.endpoints.len());
        }

        for contract in &self.pages {
            let page_id = ensure_page(definition, contract, &mut report)?;
            ensure_route(definition, contract, page_id, &mut report)?;
            let page = definition
                .pages
                .iter_mut()
                .find(|page| page.id == page_id)
                .context("刚创建的原生接口页面不存在")?;
            reconcile_page_endpoints(page, contract, &mut report)?;
        }

        report.changed = definition != &original;
        Ok(report)
    }

    fn expected_endpoints(&self) -> Result<BTreeSet<(String, String, RestMethod, String)>> {
        let mut expected = BTreeSet::new();
        for contract in &self.pages {
            for api in &contract.apis {
                expected.insert((
                    contract.page.name.clone(),
                    contract.plugin_id.clone(),
                    parse_rest_method(&api.method)?,
                    api.path.clone(),
                ));
            }
        }
        Ok(expected)
    }
}

fn ensure_page(
    definition: &mut ProgramDefinition,
    contract: &NativePageContract,
    report: &mut NativeContractReconcileReport,
) -> Result<SymbolId> {
    if let Some(page) = definition
        .pages
        .iter()
        .find(|page| page.name == contract.page.name)
    {
        return Ok(page.id);
    }
    let page_id = SymbolId::from_stable_key(&format!("native-page:{}", contract.page.name));
    definition.pages.push(PageDefinition {
        id: page_id,
        name: contract.page.name.clone(),
        title: contract.page.title.clone(),
        state: DefinitionState::Known,
        renderer: PageRendererDefinition::ConventionFile,
        endpoints: Vec::new(),
    });
    report.pages_created += 1;
    Ok(page_id)
}

fn ensure_route(
    definition: &mut ProgramDefinition,
    contract: &NativePageContract,
    page_id: SymbolId,
    report: &mut NativeContractReconcileReport,
) -> Result<()> {
    if definition
        .routes
        .iter()
        .any(|route| route.page_id == page_id)
    {
        return Ok(());
    }
    if let Some(route) = definition
        .routes
        .iter()
        .find(|route| route.path == contract.page.route)
    {
        bail!(
            "原生页面路由 {} 已属于其他页面 {}",
            contract.page.route,
            route.page_id
        );
    }
    definition.routes.push(RouteDefinition {
        id: SymbolId::from_stable_key(&format!("native-route:{}", contract.page.name)),
        name: contract.page.name.clone(),
        path: contract.page.route.clone(),
        page_id,
        state: DefinitionState::Known,
        required_permissions: Vec::new(),
    });
    report.routes_created += 1;
    Ok(())
}

fn reconcile_page_endpoints(
    page: &mut PageDefinition,
    contract: &NativePageContract,
    report: &mut NativeContractReconcileReport,
) -> Result<()> {
    for api in &contract.apis {
        let method = parse_rest_method(&api.method)?;
        if let Some(endpoint) = page
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.method == method && endpoint.path == api.path)
        {
            if matches!(
                endpoint.implementation,
                EndpointImplementationDefinition::Convention
            ) {
                bail!("原生接口不能覆盖约定契约: {} {}", api.method, api.path);
            }
            let next_implementation = EndpointImplementationDefinition::Native {
                plugin_id: contract.plugin_id.clone(),
            };
            if endpoint.title != api.label
                || endpoint.description != api.description
                || endpoint.implementation != next_implementation
            {
                endpoint.title = api.label.clone();
                endpoint.description = api.description.clone();
                endpoint.implementation = next_implementation;
                report.endpoints_updated += 1;
            }
            continue;
        }
        page.endpoints.push(PageEndpointDefinition {
            id: SymbolId::from_stable_key(&format!(
                "native-endpoint:{}:{}:{}",
                contract.plugin_id, api.method, api.path
            )),
            title: api.label.clone(),
            description: api.description.clone(),
            state: DefinitionState::Known,
            implementation: EndpointImplementationDefinition::Native {
                plugin_id: contract.plugin_id.clone(),
            },
            method,
            path: api.path.clone(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        });
        report.endpoints_created += 1;
    }
    Ok(())
}

fn parse_rest_method(value: &str) -> Result<RestMethod> {
    match value {
        "GET" => Ok(RestMethod::Get),
        "POST" => Ok(RestMethod::Post),
        "PUT" => Ok(RestMethod::Put),
        "PATCH" => Ok(RestMethod::Patch),
        "DELETE" => Ok(RestMethod::Delete),
        _ => bail!("不支持的原生接口方法: {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contributions(paths: &[(&str, &str)]) -> ContributionSet {
        ContributionSet {
            backend_page: Some(BackendPageContribution {
                name: "assets".to_owned(),
                title: "资产".to_owned(),
                route: "/assets".to_owned(),
            }),
            backend_apis: paths
                .iter()
                .enumerate()
                .map(|(index, (method, path))| BackendApiContribution {
                    id: format!("asset.api.{index}"),
                    method: (*method).to_owned(),
                    path: (*path).to_owned(),
                    label: format!("接口 {index}"),
                    description: format!("接口说明 {index}"),
                    order: index as i32,
                })
                .collect(),
            catalog_providers: Vec::new(),
        }
    }

    #[test]
    fn reconciles_native_pages_routes_and_endpoints_idempotently() -> Result<()> {
        let source = contributions(&[("GET", "/api/assets"), ("POST", "/api/assets/asset")]);
        let catalog = NativeContractCatalog::from_contributions([("asset-hub", &source)])?;
        let mut definition = ProgramDefinition::empty("aio", "AIO");

        let first = catalog.reconcile(&mut definition)?;
        assert!(first.changed);
        assert_eq!(first.pages_created, 1);
        assert_eq!(first.routes_created, 1);
        assert_eq!(first.endpoints_created, 2);
        let stable = definition.clone();

        let second = catalog.reconcile(&mut definition)?;
        assert!(!second.changed);
        assert_eq!(definition, stable);
        Ok(())
    }

    #[test]
    fn removes_stale_native_endpoint_without_touching_convention_contract() -> Result<()> {
        let initial = contributions(&[("GET", "/api/assets"), ("POST", "/api/assets/asset")]);
        let mut definition = ProgramDefinition::empty("aio", "AIO");
        NativeContractCatalog::from_contributions([("asset-hub", &initial)])?
            .reconcile(&mut definition)?;
        definition.pages[0].endpoints.push(PageEndpointDefinition {
            id: SymbolId::new(),
            title: "扩展".to_owned(),
            description: String::new(),
            state: DefinitionState::Known,
            implementation: EndpointImplementationDefinition::Convention,
            method: RestMethod::Post,
            path: "/api/assets/archive".to_owned(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        });
        let reduced = contributions(&[("GET", "/api/assets")]);

        let report = NativeContractCatalog::from_contributions([("asset-hub", &reduced)])?
            .reconcile(&mut definition)?;

        assert_eq!(report.endpoints_removed, 1);
        assert!(
            definition.pages[0]
                .endpoints
                .iter()
                .any(|endpoint| endpoint.path == "/api/assets/archive")
        );
        Ok(())
    }

    #[test]
    fn rejects_native_route_that_overwrites_convention_contract() -> Result<()> {
        let source = contributions(&[("POST", "/api/assets/archive")]);
        let catalog = NativeContractCatalog::from_contributions([("asset-hub", &source)])?;
        let mut definition = ProgramDefinition::empty("aio", "AIO");
        definition.pages.push(PageDefinition {
            id: SymbolId::new(),
            name: "assets".to_owned(),
            title: "资产".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: vec![PageEndpointDefinition {
                id: SymbolId::new(),
                title: "归档".to_owned(),
                description: String::new(),
                state: DefinitionState::Known,
                implementation: EndpointImplementationDefinition::Convention,
                method: RestMethod::Post,
                path: "/api/assets/archive".to_owned(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }],
        });

        let error = catalog
            .reconcile(&mut definition)
            .err()
            .context("原生接口覆盖约定契约必须失败")?;
        assert!(error.to_string().contains("不能覆盖约定契约"));
        Ok(())
    }
}
