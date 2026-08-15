use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};

use crate::{PageDefinition, PageEndpointDefinition, ProgramDefinition, RestMethod};

const GENERATED_MARKER: &str = ".aio-generated";

/// 将低代码接口契约同步为业务 Service 和生成 Controller。
#[derive(Clone, Debug)]
pub struct BusinessModuleManager {
    workspace_root: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BusinessModuleSyncResult {
    pub business_module: String,
    pub generated_files: Vec<String>,
    pub created_service_implementations: Vec<String>,
}

impl BusinessModuleManager {
    #[must_use]
    pub fn repository() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir
            .ancestors()
            .nth(3)
            .map(Path::to_path_buf)
            .unwrap_or(manifest_dir);
        Self { workspace_root }
    }

    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn reconcile(&self, definition: &ProgramDefinition) -> Result<BusinessModuleSyncResult> {
        validate_module_id(&definition.name)?;
        let module_dir = self.workspace_root.join("lib/biz").join(&definition.name);
        ensure_direct_child(&self.workspace_root.join("lib/biz"), &module_dir)?;
        if module_dir.exists() && !module_dir.join(GENERATED_MARKER).is_file() {
            bail!(
                "拒绝覆盖未带 {} 标记的业务模块: {}",
                GENERATED_MARKER,
                module_dir.display()
            );
        }
        fs::create_dir_all(module_dir.join("src/generated"))
            .context("创建业务模块 generated 目录失败")?;
        fs::create_dir_all(module_dir.join("src/service"))
            .context("创建业务模块 service 目录失败")?;
        fs::write(
            module_dir.join(GENERATED_MARKER),
            format!("application = {}\n", definition.name),
        )
        .context("写入业务模块生成标记失败")?;

        let pages = definition
            .pages
            .iter()
            .filter(|page| !page.endpoints.is_empty())
            .collect::<Vec<_>>();
        let mut result = BusinessModuleSyncResult {
            business_module: definition.name.clone(),
            ..BusinessModuleSyncResult::default()
        };
        write_generated(
            &module_dir,
            "Cargo.toml",
            &cargo_toml(&definition.name),
            &mut result,
        )?;
        write_generated(&module_dir, "src/lib.rs", &lib_source(), &mut result)?;
        write_generated(
            &module_dir,
            "src/generated/mod.rs",
            &generated_mod_source(&pages),
            &mut result,
        )?;
        write_generated(
            &module_dir,
            "src/service/mod.rs",
            &service_mod_source(&pages),
            &mut result,
        )?;

        let expected_page_modules = pages
            .iter()
            .map(|page| rust_identifier(&page.name))
            .collect::<BTreeSet<_>>();
        remove_stale_generated_pages(&module_dir, &expected_page_modules)?;
        for page in pages {
            let module_name = rust_identifier(&page.name);
            let page_dir = format!("src/generated/{module_name}");
            write_generated(
                &module_dir,
                &format!("{page_dir}/mod.rs"),
                &page_mod_source(page),
                &mut result,
            )?;
            write_generated(
                &module_dir,
                &format!("{page_dir}/contract.rs"),
                &service_contract_source(page),
                &mut result,
            )?;
            write_generated(
                &module_dir,
                &format!("{page_dir}/controller.rs"),
                &controller_source(page),
                &mut result,
            )?;
            remove_stale_generated_page_files(&module_dir, &page_dir)?;

            let implementation_path = module_dir.join(format!("src/service/{module_name}.rs"));
            if !implementation_path.exists() {
                fs::write(&implementation_path, service_implementation_source(page)).with_context(
                    || {
                        format!(
                            "创建业务 Service 实现失败: {}",
                            implementation_path.display()
                        )
                    },
                )?;
                result
                    .created_service_implementations
                    .push(format!("src/service/{module_name}.rs"));
            }
        }
        format_rust_sources(&module_dir, &result)?;
        result.generated_files.sort();
        result.created_service_implementations.sort();
        Ok(result)
    }
}

fn format_rust_sources(module_dir: &Path, result: &BusinessModuleSyncResult) -> Result<()> {
    let paths = result
        .generated_files
        .iter()
        .chain(&result.created_service_implementations)
        .filter(|path| path.ends_with(".rs"))
        .map(|path| module_dir.join(path))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(());
    }
    let status = match Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .args(&paths)
        .status()
    {
        Ok(status) => status,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("启动 rustfmt 失败"),
    };
    ensure!(status.success(), "rustfmt 格式化业务模块失败");
    Ok(())
}

fn write_generated(
    module_dir: &Path,
    relative_path: &str,
    source: &str,
    result: &mut BusinessModuleSyncResult,
) -> Result<()> {
    let path = module_dir.join(relative_path);
    ensure!(path.starts_with(module_dir), "生成文件越出业务模块目录");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建生成目录失败: {}", parent.display()))?;
    }
    fs::write(&path, source).with_context(|| format!("写入生成文件失败: {}", path.display()))?;
    result.generated_files.push(relative_path.to_owned());
    Ok(())
}

fn remove_stale_generated_pages(module_dir: &Path, expected: &BTreeSet<String>) -> Result<()> {
    let generated_dir = module_dir.join("src/generated");
    for entry in fs::read_dir(&generated_dir).context("读取 generated 目录失败")? {
        let entry = entry.context("读取 generated 目录项失败")?;
        if !entry
            .file_type()
            .context("读取 generated 目录项类型失败")?
            .is_dir()
        {
            continue;
        }
        let module_name = entry.file_name().to_string_lossy().into_owned();
        if expected.contains(&module_name) {
            continue;
        }
        let path = entry.path();
        ensure!(
            path.parent() == Some(generated_dir.as_path()),
            "失效生成目录越界"
        );
        fs::remove_dir_all(path).context("删除失效生成页面目录失败")?;
    }
    Ok(())
}

fn remove_stale_generated_page_files(module_dir: &Path, page_dir: &str) -> Result<()> {
    const EXPECTED_FILES: &[&str] = &["contract.rs", "controller.rs", "mod.rs"];

    let page_dir = module_dir.join(page_dir);
    for entry in fs::read_dir(&page_dir).context("读取生成页面目录失败")? {
        let entry = entry.context("读取生成页面文件失败")?;
        if !entry
            .file_type()
            .context("读取生成页面文件类型失败")?
            .is_file()
        {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if EXPECTED_FILES.contains(&file_name.as_str()) {
            continue;
        }
        let path = entry.path();
        ensure!(
            path.parent() == Some(page_dir.as_path()),
            "失效生成文件越界"
        );
        fs::remove_file(path).context("删除失效生成文件失败")?;
    }
    Ok(())
}

fn cargo_toml(application_id: &str) -> String {
    format!(
        r#"[package]
name = "az-biz-{application_id}"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
description = "Metadata-generated business services and controllers for {application_id}."
workspace = "../../.."

[dependencies]
anyhow.workspace = true
derive_more.workspace = true
dill.workspace = true
serde_json.workspace = true
studio = {{ package = "az-studio", path = "../../../app/plugins/studio", default-features = false, features = ["server"] }}

[lints]
workspace = true
"#
    )
}

fn lib_source() -> String {
    r#"#![forbid(unsafe_code)]

mod generated;
mod service;

pub use generated::register;
"#
    .to_owned()
}

fn generated_mod_source(pages: &[&PageDefinition]) -> String {
    let mut source = String::new();
    let modules = pages
        .iter()
        .map(|page| rust_identifier(&page.name))
        .collect::<BTreeSet<_>>();
    for module in modules {
        let _ = writeln!(source, "pub(crate) mod {module};");
    }
    source.push_str("\nuse dill::CatalogBuilder;\n\n");
    source.push_str("pub fn register(builder: &mut CatalogBuilder) {\n");
    for page in pages {
        let _ = writeln!(
            source,
            "    {}::register(builder);",
            rust_identifier(&page.name)
        );
    }
    source.push_str("}\n");
    source
}

fn service_mod_source(pages: &[&PageDefinition]) -> String {
    let mut source = String::new();
    let modules = pages
        .iter()
        .map(|page| rust_identifier(&page.name))
        .collect::<BTreeSet<_>>();
    for module in modules {
        let _ = writeln!(source, "pub(crate) mod {module};");
    }
    source
}

fn page_mod_source(page: &PageDefinition) -> String {
    let service_module = rust_identifier(&page.name);
    let service_impl = format!("{}Impl", service_trait_name(page));
    format!(
        r#"mod controller;
pub(crate) mod contract;

use dill::CatalogBuilder;

pub(crate) fn register(builder: &mut CatalogBuilder) {{
    builder.add::<crate::service::{service_module}::{service_impl}>();
    controller::register(builder);
}}
"#
    )
}

fn service_contract_source(page: &PageDefinition) -> String {
    let service_name = service_trait_name(page);
    let methods = endpoint_methods(page);
    let mut source =
        String::from("use studio::{ConventionEndpointFuture, ConventionEndpointRequest};\n\n");
    let _ = writeln!(source, "/// {} 领域服务契约。", page.title);
    let _ = writeln!(source, "pub trait {service_name}: Send + Sync {{");
    for (method_name, endpoint) in methods {
        let _ = writeln!(source, "    /// {}", endpoint.title);
        let _ = writeln!(source, "    fn {method_name}(");
        source.push_str("        &self,\n");
        source.push_str("        request: ConventionEndpointRequest,\n");
        source.push_str("    ) -> ConventionEndpointFuture<'_>;\n");
    }
    source.push_str("}\n");
    source
}

fn controller_source(page: &PageDefinition) -> String {
    let service_name = service_trait_name(page);
    let methods = endpoint_methods(page);
    let controllers = methods
        .iter()
        .map(|(method_name, endpoint)| {
            controller_definition_source(&service_name, method_name, endpoint)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let registrations = methods
        .iter()
        .map(|(method_name, _)| {
            format!(
                "    builder.add::<{}Controller>();",
                pascal_case(method_name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"use std::sync::Arc;

use dill::CatalogBuilder;
use studio::{{ConventionEndpointFuture, ConventionEndpointProvider, ConventionEndpointRequest}};

use super::contract::{service_name};

{controllers}
pub(crate) fn register(builder: &mut CatalogBuilder) {{
{registrations}
}}
"#
    )
}

fn controller_definition_source(
    service_name: &str,
    method_name: &str,
    endpoint: &PageEndpointDefinition,
) -> String {
    let controller_name = format!("{}Controller", pascal_case(method_name));
    let comment = rust_string(&endpoint.title);
    let endpoint_id = rust_string(&endpoint.id.to_string());

    format!(
        r#"#[dill::component]
#[dill::interface(dyn ConventionEndpointProvider)]
#[dill::scope(dill::Singleton)]
#[derive(derive_more::Debug)]
pub(crate) struct {controller_name} {{
    #[debug(skip)]
    service: Arc<dyn {service_name}>,
}}

impl ConventionEndpointProvider for {controller_name} {{
    fn comment(&self) -> &'static str {{
        {comment}
    }}

    fn endpoint_id(&self) -> &'static str {{
        {endpoint_id}
    }}

    fn handle(&self, request: ConventionEndpointRequest) -> ConventionEndpointFuture<'_> {{
        self.service.{method_name}(request)
    }}
}}
"#
    )
}

fn service_implementation_source(page: &PageDefinition) -> String {
    let service_name = service_trait_name(page);
    let implementation_name = format!("{service_name}Impl");
    let generated_module = rust_identifier(&page.name);
    let methods = endpoint_methods(page);
    let mut source = format!(
        "use anyhow::bail;\nuse studio::{{ConventionEndpointFuture, ConventionEndpointRequest}};\n\nuse crate::generated::{generated_module}::contract::{service_name};\n\n"
    );
    source.push_str("#[dill::component]\n");
    let _ = writeln!(source, "#[dill::interface(dyn {service_name})]");
    source.push_str("#[dill::scope(dill::Singleton)]\n");
    source.push_str("#[derive(Debug, Default)]\n");
    let _ = writeln!(source, "pub(crate) struct {implementation_name};\n");
    let _ = writeln!(source, "impl {service_name} for {implementation_name} {{");
    for (method_name, endpoint) in methods {
        let _ = writeln!(source, "    fn {method_name}(");
        source.push_str("        &self,\n");
        source.push_str("        request: ConventionEndpointRequest,\n");
        source.push_str("    ) -> ConventionEndpointFuture<'_> {\n");
        source.push_str("        Box::pin(async move {\n");
        source.push_str("            let _ = request;\n");
        let _ = writeln!(
            source,
            "            bail!({})",
            rust_string(&format!("{}尚未实现", endpoint.title))
        );
        source.push_str("        })\n    }\n\n");
    }
    source.push_str("}\n");
    source
}

fn endpoint_methods(page: &PageDefinition) -> Vec<(String, &PageEndpointDefinition)> {
    let mut used = BTreeMap::<String, usize>::new();
    page.endpoints
        .iter()
        .map(|endpoint| {
            let mut segments = endpoint
                .path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .skip(2)
                .map(|segment| segment.trim_matches(['{', '}']))
                .collect::<Vec<_>>();
            if segments.is_empty() {
                segments.push("execute");
            }
            let method = match endpoint.method {
                RestMethod::Get => "get",
                RestMethod::Post => "post",
                RestMethod::Put => "put",
                RestMethod::Patch => "patch",
                RestMethod::Delete => "delete",
            };
            let base = rust_identifier(&format!("{method}_{}", segments.join("_")));
            let count = used.entry(base.clone()).or_default();
            let resolved = if *count == 0 {
                base
            } else {
                format!("{base}_{}", endpoint.id.to_string().replace('-', "_"))
            };
            *count += 1;
            (resolved, endpoint)
        })
        .collect()
}

fn service_trait_name(page: &PageDefinition) -> String {
    format!("{}Service", pascal_case(&page.name))
}

fn rust_identifier(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('_');
            }
            output.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if output.is_empty() {
        output.push_str("module");
    }
    if output.as_bytes()[0].is_ascii_digit() {
        output.insert_str(0, "module_");
    }
    if is_rust_keyword(&output) {
        output.push('_');
    }
    output
}

fn pascal_case(value: &str) -> String {
    rust_identifier(value)
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();
            match characters.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), characters.as_str()),
                None => String::new(),
            }
        })
        .collect()
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn validate_module_id(value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "业务模块标识不能为空");
    ensure!(
        value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase()),
        "业务模块标识必须以小写字母开头"
    );
    ensure!(
        value.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'),
        "业务模块标识只能包含小写字母、数字和连字符"
    );
    Ok(())
}

fn ensure_direct_child(parent: &Path, child: &Path) -> Result<()> {
    ensure!(child.parent() == Some(parent), "业务模块路径越出 lib/biz");
    Ok(())
}

fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
    )
}

#[cfg(test)]
mod tests {
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
        let contract = fs::read_to_string(module.join("src/generated/orders/contract.rs"))?;

        assert!(controller.contains("ConventionEndpointProvider"));
        assert!(controller.contains("#[dill::component]"));
        assert!(controller.contains("#[derive(derive_more::Debug)]"));
        assert!(controller.contains("#[debug(skip)]"));
        assert!(controller.contains("self.service.post_submit(request)"));
        assert!(!controller.contains("fn key"));
        assert!(!controller.contains("fn name"));
        assert!(!controller.contains("module_path!"));
        assert!(contract.contains("trait OrdersService"));
        assert!(!contract.contains("std::fmt::Debug"));
        assert!(!contract.contains("Debug + Send + Sync"));
        assert_eq!(result.created_service_implementations.len(), 1);

        let implementation = module.join("src/service/orders.rs");
        fs::write(&implementation, "// 人工实现\n")?;
        let stale_generated_service = module.join("src/generated/orders/service.rs");
        fs::write(&stale_generated_service, "// 失效生成文件\n")?;
        manager.reconcile(&definition())?;
        assert_eq!(fs::read_to_string(implementation)?, "// 人工实现\n");
        assert!(!stale_generated_service.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn removes_generated_page_when_endpoint_metadata_is_deleted() -> Result<()> {
        let root = test_root("metadata-deletion");
        let manager = BusinessModuleManager::new(root.clone());
        let mut definition = definition();
        manager.reconcile(&definition)?;

        let module = root.join("lib/biz/example-app");
        let generated_page = module.join("src/generated/orders");
        let service_implementation = module.join("src/service/orders.rs");
        assert!(generated_page.is_dir());
        assert!(service_implementation.is_file());

        definition.pages[0].endpoints.clear();
        manager.reconcile(&definition)?;

        assert!(!generated_page.exists());
        assert!(service_implementation.is_file());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
