use std::process;

use crate::{
    DefinitionState, PageDefinition, PageEndpointDefinition, PageRendererDefinition, SymbolId,
};

use super::*;

fn test_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "aio-biz-{name}-{}-{}",
        process::id(),
        az_plugin_core::timestamp_ms()
    ))
}

fn definition() -> ProgramDefinition {
    let mut definition = ProgramDefinition::empty("example-app", "示例应用");
    definition.pages.push(PageDefinition {
        id: SymbolId::new(),
        name: "orders".to_owned(),
        title: "订单".to_owned(),
        state: DefinitionState::Known,
        renderer: PageRendererDefinition::ConventionFile,
        endpoints: vec![PageEndpointDefinition {
            id: SymbolId::new(),
            title: "提交订单".to_owned(),
            description: String::new(),
            state: DefinitionState::Known,
            method: RestMethod::Post,
            path: "/api/orders/submit".to_owned(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }],
    });
    definition
}

#[test]
fn generates_service_and_controller_without_string_plugin_identity() -> Result<()> {
    let root = test_root("layout");
    let manager = BusinessModuleManager::new(root.clone());
    let result = manager.reconcile(&definition())?;
    let module = root.join("lib/biz/example-app");
    let controller = fs::read_to_string(module.join("src/generated/orders/controller.rs"))?;
    let contract = fs::read_to_string(module.join("src/generated/orders/service.rs"))?;
    let manifest = fs::read_to_string(module.join("Cargo.toml"))?;

    assert!(controller.contains("ConventionEndpointProvider"));
    assert!(controller.contains("#[dill::component]"));
    assert!(!controller.contains("derive_more::Debug"));
    assert!(!controller.contains("fn comment"));
    assert!(controller.contains("self.service.post_submit(request)"));
    assert!(!controller.contains("fn key"));
    assert!(!controller.contains("fn name"));
    assert!(!controller.contains("module_path!"));
    assert!(!manifest.contains("derive_more"));
    assert!(!manifest.contains("serde_json"));
    assert!(contract.contains("trait OrdersService"));
    assert!(
        fs::read_to_string(module.join("src/generated/orders/model.rs"))?
            .contains("EndpointRequest")
    );
    assert!(
        fs::read_to_string(module.join("src/generated/orders/util.rs"))?.contains("endpoint_id")
    );
    assert!(!contract.contains("std::fmt::Debug"));
    assert!(!contract.contains("Debug + Send + Sync"));
    assert_eq!(result.created_service_implementations.len(), 1);

    let implementation = module.join("src/generated/orders/service_impl.rs");
    let legacy_implementation = fs::read_to_string(&implementation)?
        .replace(SERVICE_STUB_COMMENT, "")
        .replace("提交订单尚未实现", "旧提示尚未实现");
    assert_eq!(
        service_stub_shape(&legacy_implementation),
        service_stub_shape(&service_implementation_source(&definition().pages[0]))
    );
    fs::write(&implementation, legacy_implementation)?;
    fs::remove_file(module.join(SERVICE_STUB_MANIFEST))?;
    manager.reconcile(&definition())?;
    assert!(fs::read_to_string(&implementation)?.starts_with(SERVICE_STUB_COMMENT));

    fs::write(&implementation, "// 人工实现\n")?;
    manager.reconcile(&definition())?;
    assert_eq!(fs::read_to_string(implementation)?, "// 人工实现\n");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn removes_generated_page_and_untouched_service_stub_when_metadata_is_deleted() -> Result<()> {
    let root = test_root("metadata-deletion");
    let manager = BusinessModuleManager::new(root.clone());
    let mut definition = definition();
    manager.reconcile(&definition)?;

    let module = root.join("lib/biz/example-app");
    let generated_page = module.join("src/generated/orders");
    let service_implementation = module.join("src/generated/orders/service_impl.rs");
    assert!(generated_page.is_dir());
    assert!(service_implementation.is_file());

    definition.pages[0].endpoints.clear();
    manager.reconcile(&definition)?;

    assert!(!generated_page.exists());
    assert!(!service_implementation.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn preserves_modified_service_implementation_when_metadata_is_deleted() -> Result<()> {
    let root = test_root("manual-service");
    let manager = BusinessModuleManager::new(root.clone());
    let mut definition = definition();
    manager.reconcile(&definition)?;

    let module = root.join("lib/biz/example-app");
    let service_implementation = module.join("src/generated/orders/service_impl.rs");
    fs::write(&service_implementation, "// 人工实现\n")?;

    definition.pages[0].endpoints.clear();
    manager.reconcile(&definition)?;

    assert_eq!(fs::read_to_string(service_implementation)?, "// 人工实现\n");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn repeated_reconcile_does_not_rewrite_unchanged_generated_files() -> Result<()> {
    let root = test_root("idempotent");
    let manager = BusinessModuleManager::new(root.clone());
    let definition = definition();
    manager.reconcile(&definition)?;

    let module = root.join("lib/biz/example-app");
    let tracked = [
        module.join(GENERATED_MARKER),
        module.join("src/generated/orders/controller.rs"),
        module.join("src/generated/orders/service_impl.rs"),
        module.join(SERVICE_STUB_MANIFEST),
    ];
    let modified_before = tracked
        .iter()
        .map(fs::metadata)
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|metadata| metadata.modified())
        .collect::<std::io::Result<Vec<_>>>()?;

    let result = manager.reconcile(&definition)?;
    let modified_after = tracked
        .iter()
        .map(fs::metadata)
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|metadata| metadata.modified())
        .collect::<std::io::Result<Vec<_>>>()?;

    assert!(
        result.changed_rust_sources.is_empty(),
        "重复变化的生成文件: {result:#?}"
    );
    assert_eq!(modified_before, modified_after);
    fs::remove_dir_all(root)?;
    Ok(())
}
