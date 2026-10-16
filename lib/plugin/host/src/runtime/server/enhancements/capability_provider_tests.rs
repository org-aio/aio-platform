use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use super::capabilities::*;

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
