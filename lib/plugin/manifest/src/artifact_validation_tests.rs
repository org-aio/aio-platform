use std::fs;

use anyhow::{Context, Result, ensure};

use crate::{
    PageBody, PageDefinition, SceneDefinition, WasmArtifactValidator, validate_repository,
};

// 契约替身只用于验证调用方注入和公共检查，不表示它是新的 Wasm 引擎。
struct ExternalValidator(Vec<PageDefinition>);

impl WasmArtifactValidator for ExternalValidator {
    fn validate(&self, bytes: &[u8]) -> Result<Vec<PageDefinition>> {
        ensure!(bytes == b"external-validator-fixture", "产物被拒绝");
        Ok(self.0.clone())
    }
}

fn page() -> PageDefinition {
    PageDefinition {
        id: "external".into(),
        label: "External".into(),
        icon: None,
        scene: SceneDefinition {
            id: "tools".into(),
            label: "Tools".into(),
        },
        menu_path: vec![],
        required_permission: None,
        body: PageBody::Text {
            title: "External".into(),
            content: "Fixture".into(),
        },
    }
}

fn repository() -> Result<tempfile::TempDir> {
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join("aio-plugin.toml"),
        "[plugin.runtime]\nkind='wasm-component'\nartifact='plugin.wasm'\n[[plugin.subplugins]]\nid='external'\npages=['external']\n",
    )?;
    fs::write(
        root.path().join("plugin.wasm"),
        b"external-validator-fixture",
    )?;
    Ok(root)
}

#[test]
fn explicit_executor_validates_without_builtin_wasmtime_and_rejects_corruption() -> Result<()> {
    let root = repository()?;
    let validator = ExternalValidator(vec![page()]);
    let report = validate_repository(root.path(), Some(&validator))?;
    assert_eq!(report.page_count, 1);
    // 即使编译了内置引擎，也不能用它替代缺失的已选执行器。
    assert!(validate_repository(root.path(), None).is_err());
    fs::write(root.path().join("plugin.wasm"), b"corrupted")?;
    let error = validate_repository(root.path(), Some(&validator))
        .err()
        .context("损坏产物未被拒绝")?;
    assert!(error.to_string().contains("产物被拒绝"));
    Ok(())
}

#[test]
fn external_executor_cannot_bypass_page_models_declarations_or_frontend_files() -> Result<()> {
    let root = repository()?;
    let mut blank = page();
    blank.label.clear();
    let mut undeclared = page();
    undeclared.id = "other".into();
    let mut frontend = page();
    frontend.body = PageBody::Frontend {
        entry: "index.html".into(),
    };
    for pages in [
        vec![],
        vec![blank],
        vec![undeclared],
        vec![frontend.clone()],
    ] {
        let validator = ExternalValidator(pages);
        assert!(validate_repository(root.path(), Some(&validator)).is_err());
    }
    let manifest = root.path().join("aio-plugin.toml");
    let text = fs::read_to_string(&manifest)?;
    fs::write(manifest, format!("{text}\n[plugin.frontend]\npath='web'\n"))?;
    fs::create_dir(root.path().join("web"))?;
    fs::write(root.path().join("web/other.html"), "<!doctype html>")?;
    let validator = ExternalValidator(vec![frontend]);
    assert!(validate_repository(root.path(), Some(&validator)).is_err());
    fs::write(root.path().join("web/index.html"), "<!doctype html>")?;
    assert_eq!(
        validate_repository(root.path(), Some(&validator))?.page_count,
        1
    );
    Ok(())
}
