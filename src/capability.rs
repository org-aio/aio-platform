use std::{
    any::{Any, TypeId},
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use crate::{CapabilityCatalog as ProgramCapabilityCatalog, CapabilityContract};
use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait CapabilityProvider: Any + Send + Sync {
    fn contract(&self) -> CapabilityContract;

    async fn execute(&self, operation: &str, input: Value) -> Result<Value>;
}

pub type DynCapabilityProvider = Arc<dyn CapabilityProvider>;

#[derive(Clone, Default)]
pub struct CapabilityCatalog {
    providers: BTreeMap<String, DynCapabilityProvider>,
    contracts: ProgramCapabilityCatalog,
}

impl CapabilityCatalog {
    pub fn new(providers: Vec<DynCapabilityProvider>) -> Result<Self> {
        let mut indexed = BTreeMap::new();
        let mut contracts = BTreeMap::new();
        let mut provider_types = HashSet::<TypeId>::new();
        for provider in providers {
            let type_id = provider.as_ref().type_id();
            if !provider_types.insert(type_id) {
                bail!("Capability Provider 类型重复: {type_id:?}");
            }
            let contract = provider.contract();
            if contract.canonical_id.trim().is_empty() {
                bail!("Capability canonical_id 不能为空");
            }
            if contract.operations.is_empty() {
                bail!(
                    "Capability 必须声明至少一个 operation: {}",
                    contract.canonical_id
                );
            }
            let id = contract.canonical_id.clone();
            if indexed.insert(id.clone(), provider).is_some() {
                bail!("Capability 注册重复: {id}");
            }
            contracts.insert(id, contract);
        }
        Ok(Self {
            providers: indexed,
            contracts: ProgramCapabilityCatalog {
                capabilities: contracts,
            },
        })
    }

    #[must_use]
    pub fn program_catalog(&self) -> ProgramCapabilityCatalog {
        self.contracts.clone()
    }

    pub async fn execute(
        &self,
        capability_id: &str,
        operation: &str,
        input: Value,
    ) -> Result<Value> {
        let provider = self
            .providers
            .get(capability_id)
            .ok_or_else(|| anyhow::anyhow!("Capability 未注册: {capability_id}"))?;
        let contract = self
            .contracts
            .capabilities
            .get(capability_id)
            .ok_or_else(|| anyhow::anyhow!("Capability 契约缺失: {capability_id}"))?;
        if !contract.operations.contains_key(operation) {
            bail!("Capability operation 未注册: {capability_id}.{operation}");
        }
        provider.execute(operation, input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CapabilityOperationContract, EffectKind};

    struct EchoCapability;

    #[async_trait]
    impl CapabilityProvider for EchoCapability {
        fn contract(&self) -> CapabilityContract {
            CapabilityContract {
                canonical_id: "test.echo".to_owned(),
                operations: BTreeMap::from([(
                    "execute".to_owned(),
                    CapabilityOperationContract {
                        inputs: BTreeMap::new(),
                        outputs: BTreeMap::new(),
                        effects: vec![EffectKind::Capability],
                    },
                )]),
            }
        }

        async fn execute(&self, operation: &str, input: Value) -> Result<Value> {
            assert_eq!(operation, "execute");
            Ok(input)
        }
    }

    #[tokio::test]
    async fn contract_and_execution_share_one_provider() {
        let catalog = CapabilityCatalog::new(vec![Arc::new(EchoCapability)]).unwrap();
        assert!(
            catalog
                .program_catalog()
                .capabilities
                .contains_key("test.echo")
        );
        assert_eq!(
            catalog
                .execute("test.echo", "execute", Value::Bool(true))
                .await
                .unwrap(),
            Value::Bool(true)
        );
    }
}
