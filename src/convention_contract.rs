use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

use crate::{EndpointImplementationDefinition, ProgramDefinition};

const CONTRACT_DIRECTORY_ANCHOR: &str = "contract_directory.rs";

#[derive(Clone, Debug)]
pub struct ConventionContractManager {
    contracts_dir: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConventionContractSyncResult {
    pub created: Vec<String>,
    pub removed: Vec<String>,
    pub retained: Vec<String>,
}

impl ConventionContractManager {
    #[must_use]
    pub fn workspace_app() -> Self {
        let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self {
            contracts_dir: app_dir.join("src/contracts"),
        }
    }

    #[must_use]
    pub fn new(contracts_dir: PathBuf) -> Self {
        Self { contracts_dir }
    }

    pub fn reconcile(
        &self,
        definition: &ProgramDefinition,
    ) -> Result<ConventionContractSyncResult> {
        fs::create_dir_all(&self.contracts_dir)
            .with_context(|| format!("创建约定接口目录失败: {}", self.contracts_dir.display()))?;
        let expected = convention_endpoint_ids(definition)
            .into_iter()
            .map(|endpoint_id| (contract_file_name(&endpoint_id), endpoint_id))
            .collect::<BTreeSet<_>>();
        let expected_names = expected
            .iter()
            .map(|(file_name, _)| file_name.clone())
            .collect::<BTreeSet<_>>();
        let mut result = ConventionContractSyncResult::default();

        for (file_name, endpoint_id) in expected {
            let path = self.contracts_dir.join(&file_name);
            if path.exists() {
                result.retained.push(file_name);
                continue;
            }
            let source = convention_contract_source(&endpoint_id);
            fs::write(&path, source)
                .with_context(|| format!("写入约定接口文件失败: {}", path.display()))?;
            result.created.push(file_name);
        }

        for entry in fs::read_dir(&self.contracts_dir)
            .with_context(|| format!("读取约定接口目录失败: {}", self.contracts_dir.display()))?
        {
            let entry = entry.context("读取约定接口目录项失败")?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name == CONTRACT_DIRECTORY_ANCHOR || expected_names.contains(&file_name) {
                continue;
            }
            ensure_inside_contracts(&self.contracts_dir, &path)?;
            fs::remove_file(&path)
                .with_context(|| format!("删除失效约定接口文件失败: {}", path.display()))?;
            result.removed.push(file_name);
        }
        result.created.sort();
        result.removed.sort();
        result.retained.sort();
        Ok(result)
    }
}

fn convention_endpoint_ids(definition: &ProgramDefinition) -> BTreeSet<String> {
    definition
        .pages
        .iter()
        .flat_map(|page| &page.endpoints)
        .filter(|endpoint| {
            matches!(
                endpoint.implementation,
                EndpointImplementationDefinition::Convention
            )
        })
        .map(|endpoint| endpoint.id.to_string())
        .collect()
}

fn contract_file_name(endpoint_id: &str) -> String {
    format!("contract_{}.rs", endpoint_id.replace('-', "_"))
}

fn ensure_inside_contracts(contracts_dir: &Path, path: &Path) -> Result<()> {
    let contracts_dir = contracts_dir
        .canonicalize()
        .context("解析约定接口目录失败")?;
    let path = path.canonicalize().context("解析约定接口文件失败")?;
    ensure!(path.starts_with(contracts_dir), "约定接口文件越出专用目录");
    Ok(())
}

fn convention_contract_source(endpoint_id: &str) -> String {
    format!(
        r#"use std::sync::Arc;

use anyhow::{{Result, bail}};
use rudi::Singleton;
use serde_json::Value;
use studio::{{
    ConventionEndpointFuture, ConventionEndpointProvider, ConventionEndpointRequest,
    DynConventionEndpointProvider,
}};

#[derive(Clone, Debug, Default)]
struct Endpoint;

impl ConventionEndpointProvider for Endpoint {{
    fn key(&self) -> &'static str {{
        module_path!()
    }}

    fn endpoint_id(&self) -> &'static str {{
        "{endpoint_id}"
    }}

    fn handle(&self, request: ConventionEndpointRequest) -> ConventionEndpointFuture<'_> {{
        Box::pin(handle(request))
    }}
}}

// 在这里实现约定接口业务，路由、方法与统一响应由 Studio 运行时负责。
async fn handle(request: ConventionEndpointRequest) -> Result<Value> {{
    let _ = request;
    bail!("约定接口尚未实现")
}}

#[Singleton(name = module_path!())]
fn convention_endpoint() -> DynConventionEndpointProvider {{
    Arc::new(Endpoint)
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use std::process;

    use crate::{
        DefinitionState, EndpointImplementationDefinition, PageDefinition, PageEndpointDefinition,
        PageRendererDefinition, RestMethod, SymbolId,
    };

    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "aio-contract-{name}-{}-{}",
            process::id(),
            az_plugin_core::timestamp_ms()
        ))
    }

    fn definition(
        endpoint_id: SymbolId,
        implementation: EndpointImplementationDefinition,
    ) -> ProgramDefinition {
        let mut definition = ProgramDefinition::empty("aio", "AIO");
        definition.pages.push(PageDefinition {
            id: SymbolId::new(),
            name: "orders".to_owned(),
            title: "订单".to_owned(),
            state: DefinitionState::Known,
            renderer: PageRendererDefinition::ConventionFile,
            endpoints: vec![PageEndpointDefinition {
                id: endpoint_id,
                title: "提交订单".to_owned(),
                description: String::new(),
                state: DefinitionState::Known,
                implementation,
                method: RestMethod::Post,
                path: "/api/orders".to_owned(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }],
        });
        definition
    }

    #[test]
    fn creates_preserves_and_removes_convention_contract_file() -> Result<()> {
        let dir = test_dir("lifecycle");
        let manager = ConventionContractManager::new(dir.clone());
        let endpoint_id = SymbolId::new();
        let mut definition = definition(endpoint_id, EndpointImplementationDefinition::Convention);

        let created = manager.reconcile(&definition)?;
        assert_eq!(created.created.len(), 1);
        let path = dir.join(&created.created[0]);
        let custom_source = format!("{}\n// 人工实现保留", fs::read_to_string(&path)?);
        fs::write(&path, &custom_source)?;

        let retained = manager.reconcile(&definition)?;
        assert_eq!(retained.retained, created.created);
        assert_eq!(fs::read_to_string(&path)?, custom_source);

        definition.pages[0].endpoints.clear();
        let removed = manager.reconcile(&definition)?;
        assert_eq!(removed.removed.len(), 1);
        assert!(!path.exists());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn native_endpoint_does_not_create_extension_source() -> Result<()> {
        let dir = test_dir("native");
        let manager = ConventionContractManager::new(dir.clone());
        let definition = definition(
            SymbolId::new(),
            EndpointImplementationDefinition::Native {
                plugin_id: "orders".to_owned(),
            },
        );

        let result = manager.reconcile(&definition)?;

        assert!(result.created.is_empty());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn generated_contract_uses_rust_qualified_provider_identity() {
        let source = convention_contract_source("5cbf910c-05af-4537-94d3-673c3b4c444b");

        assert!(source.contains("module_path!()"));
        assert!(source.contains("ConventionEndpointProvider"));
        assert!(source.contains("约定接口尚未实现"));
    }
}
