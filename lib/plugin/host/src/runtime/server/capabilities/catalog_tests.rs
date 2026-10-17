use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use super::*;

struct First;
struct Second;

macro_rules! provider {
    ($implementation:ty) => {
        #[async_trait::async_trait]
        impl CapabilityProvider for $implementation {
            fn capabilities(&self) -> Vec<RuntimeCapability> {
                vec![RuntimeCapability {
                    path: "/v1/test/read".into(),
                    description: "测试能力".into(),
                    effect: CapabilityEffect::Read,
                    input_schema: schemars::schema_for!(Value),
                    output_schema: schemars::schema_for!(Value),
                }]
            }
            async fn invoke(&self, _: &CapabilityRequest) -> Result<Value> {
                anyhow::bail!("装配检查不能执行能力")
            }
        }
    };
}
provider!(First);
provider!(Second);

struct Dynamic;
struct Definitions(Vec<RuntimeCapability>);

#[async_trait::async_trait]
impl CapabilityProvider for Dynamic {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        vec![]
    }
    fn snapshot(&self) -> Option<Arc<dyn CapabilityProvider>> {
        Some(Arc::new(Definitions(vec![])))
    }
    async fn invoke(&self, _: &CapabilityRequest) -> Result<Value> {
        anyhow::bail!("只能执行已发布快照")
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for Definitions {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        self.0.clone()
    }
    async fn invoke(&self, _: &CapabilityRequest) -> Result<Value> {
        anyhow::bail!("装配检查不能执行能力")
    }
}

#[test]
fn dynamic_publication_is_atomic_and_reserves_retired_contracts() -> Result<()> {
    let mut services = services();
    services
        .add_value(Dynamic)
        .bind::<dyn CapabilityProvider, Dynamic>();
    let catalog = services.build();
    let runtime = catalog.get_one::<RuntimeCapabilities>()?;
    runtime.validate()?;
    let mut capability = First.capabilities().remove(0);
    let mut committed = false;
    assert!(
        runtime
            .publish::<Dynamic>(Arc::new(Definitions(vec![capability.clone()])), || {
                committed = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!committed, "路径冲突不能调用提交动作");
    capability.path = "/v1/dynamic/read".into();
    let mut invalid = capability.clone();
    invalid.input_schema = serde_json::json!({"$ref":"file:///unavailable-schema"}).try_into()?;
    assert!(
        runtime
            .publish::<Dynamic>(Arc::new(Definitions(vec![invalid])), || {
                panic!("无效 schema 不能提交候选")
            })
            .is_err()
    );
    assert_eq!(runtime.catalog("test")?.len(), 1);
    assert!(
        runtime
            .publish::<First>(Arc::new(Definitions(vec![capability.clone()])), || Ok(()))
            .is_err()
    );
    assert!(
        runtime
            .publish::<Dynamic>(
                Arc::new(Definitions(vec![capability.clone()])),
                || anyhow::bail!("提交被取消")
            )
            .is_err()
    );
    assert_eq!(runtime.catalog("test")?.len(), 1);
    runtime.publish::<Dynamic>(Arc::new(Definitions(vec![capability.clone()])), || Ok(()))?;
    assert_eq!(runtime.catalog("test")?.len(), 2);
    runtime.publish::<Dynamic>(Arc::new(Definitions(vec![])), || Ok(()))?;
    assert_eq!(runtime.catalog("test")?.len(), 1);
    let mut incompatible = capability.clone();
    incompatible.effect = CapabilityEffect::Write;
    assert!(
        runtime
            .publish::<Dynamic>(Arc::new(Definitions(vec![incompatible])), || Ok(()))
            .is_err()
    );
    assert_eq!(runtime.catalog("test")?.len(), 1);
    runtime.publish::<Dynamic>(Arc::new(Definitions(vec![capability.clone()])), || Ok(()))?;
    assert!(
        runtime
            .publish::<Dynamic>(
                Arc::new(Definitions(vec![capability.clone(), capability])),
                || Ok(())
            )
            .is_err()
    );
    assert_eq!(runtime.catalog("test")?.len(), 2);
    Ok(())
}

#[test]
fn failed_candidate_does_not_consume_contract_history_or_allow_unbounded_growth() -> Result<()> {
    let runtime = RuntimeCapabilities::new(vec![Arc::new(Dynamic)]);
    runtime.validate()?;
    let definitions = (0..4097)
        .map(|index| {
            let mut capability = First.capabilities().remove(0);
            capability.path = format!("/v1/test/path{index}");
            capability
        })
        .collect();
    assert!(
        runtime
            .publish::<Dynamic>(Arc::new(Definitions(definitions)), || Ok(()))
            .is_err()
    );
    assert!(runtime.catalog("test")?.is_empty());
    let mut capability = First.capabilities().remove(0);
    capability.path = "/v1/test/path0".into();
    capability.effect = CapabilityEffect::Write;
    runtime.publish::<Dynamic>(Arc::new(Definitions(vec![capability])), || Ok(()))?;
    assert_eq!(runtime.catalog("test")?.len(), 1);
    Ok(())
}

fn services() -> dill::CatalogBuilder {
    let mut services = dill::CatalogBuilder::new();
    services
        .add::<RuntimeCapabilities>()
        .add_value(First)
        .bind::<dyn CapabilityProvider, First>();
    services
}

#[test]
fn provider_type_and_path_conflicts_are_rejected_without_registration_order_fallback() -> Result<()>
{
    // Dill 的 builder 克隆共享单例缓存；不同装配图必须各自创建 builder。
    let catalog = services().build();
    let runtime = catalog.get_one::<RuntimeCapabilities>()?;
    runtime.validate()?;
    assert_eq!(runtime.catalog("test")?.len(), 1);
    let original = Arc::downgrade(&runtime);
    drop(runtime);
    assert!(original.upgrade().is_some());
    drop(catalog);
    assert!(original.upgrade().is_none(), "能力目录不应强持有所属容器");
    let mut duplicated = services();
    duplicated.bind::<dyn CapabilityProvider, First>();
    assert!(
        duplicated
            .build()
            .get_one::<RuntimeCapabilities>()?
            .validate()
            .is_err()
    );
    let mut ambiguous = services();
    ambiguous
        .add_value(Second)
        .bind::<dyn CapabilityProvider, Second>();
    assert!(
        ambiguous
            .build()
            .get_one::<RuntimeCapabilities>()?
            .validate()
            .is_err()
    );
    Ok(())
}
