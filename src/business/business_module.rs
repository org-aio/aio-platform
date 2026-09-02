use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PageDefinition, PageEndpointDefinition, ProgramDefinition, RestMethod};

#[path = "module_validation.rs"]
mod module_validation;
use module_validation::{ensure_direct_child, is_rust_keyword, validate_module_id};

#[path = "source_format.rs"]
mod business_module_source_format;
use business_module_source_format::write_rust_source_if_changed;

const GENERATED_MARKER: &str = ".aio-generated";
const SERVICE_STUB_MANIFEST: &str = "src/generated/service-stubs.json";
const SERVICE_STUB_COMMENT: &str =
    "// AIO 根据元数据生成的 Service 实现起点；内容修改后自动转为人工所有。\n";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ServiceStubManifest {
    #[serde(default)]
    files: BTreeMap<String, String>,
}

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
    changed_rust_sources: BTreeSet<String>,
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
        write_if_changed(
            &module_dir.join(GENERATED_MARKER),
            format!("application = {}\n", definition.name).as_bytes(),
        )?;

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

        let expected_page_modules = pages
            .iter()
            .map(|page| rust_identifier(&page.name))
            .collect::<BTreeSet<_>>();
        let expected_service_paths = expected_page_modules
            .iter()
            .map(|module_name| format!("src/generated/{module_name}/service_impl.rs"))
            .collect::<BTreeSet<_>>();
        let mut service_stub_manifest = load_service_stub_manifest(&module_dir)?;
        remove_stale_generated_pages(&module_dir, &expected_page_modules, &service_stub_manifest)?;
        remove_stale_service_stubs(
            &module_dir,
            &expected_service_paths,
            &mut service_stub_manifest,
        )?;
        let mut owned_service_stubs = BTreeSet::new();
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
                &format!("{page_dir}/service.rs"),
                &service_contract_source(page),
                &mut result,
            )?;
            write_generated(
                &module_dir,
                &format!("{page_dir}/controller.rs"),
                &controller_source(page),
                &mut result,
            )?;
            write_generated(
                &module_dir,
                &format!("{page_dir}/model.rs"),
                &model_source(),
                &mut result,
            )?;
            write_generated(
                &module_dir,
                &format!("{page_dir}/util.rs"),
                &util_source(),
                &mut result,
            )?;

            let implementation_relative = format!("{page_dir}/service_impl.rs");
            let implementation_path = module_dir.join(&implementation_relative);
            let implementation_source = service_implementation_source(page);
            let implementation_exists = implementation_path.exists();
            if generated_service_stub_is_owned(
                &implementation_path,
                &implementation_relative,
                &implementation_source,
                &mut service_stub_manifest,
            )? {
                write_generated(
                    &module_dir,
                    &implementation_relative,
                    &implementation_source,
                    &mut result,
                )?;
                owned_service_stubs.insert(implementation_relative.clone());
            }
            if !implementation_exists {
                result
                    .created_service_implementations
                    .push(implementation_relative);
            }
            remove_stale_generated_page_files(&module_dir, &page_dir)?;
        }
        update_service_stub_manifest(
            &module_dir,
            &owned_service_stubs,
            &mut service_stub_manifest,
        )?;
        write_service_stub_manifest(&module_dir, &service_stub_manifest)?;
        result
            .generated_files
            .push(SERVICE_STUB_MANIFEST.to_owned());
        result.generated_files.sort();
        result.created_service_implementations.sort();
        Ok(result)
    }
}

fn load_service_stub_manifest(module_dir: &Path) -> Result<ServiceStubManifest> {
    let path = module_dir.join(SERVICE_STUB_MANIFEST);
    if !path.is_file() {
        return Ok(ServiceStubManifest::default());
    }
    serde_json::from_slice(
        &fs::read(&path)
            .with_context(|| format!("读取 Service 骨架所有权清单失败: {}", path.display()))?,
    )
    .with_context(|| format!("解析 Service 骨架所有权清单失败: {}", path.display()))
}

fn remove_stale_service_stubs(
    module_dir: &Path,
    expected_paths: &BTreeSet<String>,
    manifest: &mut ServiceStubManifest,
) -> Result<()> {
    let stale_paths = manifest
        .files
        .keys()
        .filter(|path| !expected_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    for relative_path in stale_paths {
        let path = module_dir.join(&relative_path);
        ensure!(path.starts_with(module_dir), "Service 骨架路径越出业务模块");
        if path.is_file() {
            let source = fs::read(&path)
                .with_context(|| format!("读取 Service 骨架失败: {}", path.display()))?;
            let expected_hash = manifest
                .files
                .get(&relative_path)
                .cloned()
                .unwrap_or_default();
            if source_hash(&source) == expected_hash {
                fs::remove_file(&path)
                    .with_context(|| format!("删除失效 Service 骨架失败: {}", path.display()))?;
            }
        }
        manifest.files.remove(&relative_path);
    }
    Ok(())
}

fn generated_service_stub_is_owned(
    path: &Path,
    relative_path: &str,
    generated_source: &str,
    manifest: &mut ServiceStubManifest,
) -> Result<bool> {
    if !path.is_file() {
        return Ok(true);
    }
    let current =
        fs::read(path).with_context(|| format!("读取业务 Service 实现失败: {}", path.display()))?;
    if let Some(expected_hash) = manifest.files.get(relative_path) {
        if source_hash(&current) == *expected_hash {
            return Ok(true);
        }
        manifest.files.remove(relative_path);
        return Ok(false);
    }
    let current = String::from_utf8_lossy(&current);
    Ok(service_stub_shape(&current) == service_stub_shape(generated_source))
}

fn update_service_stub_manifest(
    module_dir: &Path,
    owned_paths: &BTreeSet<String>,
    manifest: &mut ServiceStubManifest,
) -> Result<()> {
    for relative_path in owned_paths {
        let path = module_dir.join(relative_path);
        let source = fs::read(&path)
            .with_context(|| format!("读取已格式化 Service 骨架失败: {}", path.display()))?;
        manifest
            .files
            .insert(relative_path.clone(), source_hash(&source));
    }
    Ok(())
}

fn write_service_stub_manifest(module_dir: &Path, manifest: &ServiceStubManifest) -> Result<()> {
    let path = module_dir.join(SERVICE_STUB_MANIFEST);
    let source =
        serde_json::to_vec_pretty(manifest).context("序列化 Service 骨架所有权清单失败")?;
    write_if_changed(&path, &source).map(|_| ())
}

fn source_hash(source: &[u8]) -> String {
    format!("{:x}", Sha256::digest(source))
}

fn service_stub_shape(source: &str) -> String {
    let mut imports = Vec::new();
    let mut body = String::new();
    for line in source.lines() {
        if line.starts_with(SERVICE_STUB_COMMENT.trim()) {
            continue;
        }
        let line = line.trim();
        if line.starts_with("use ") {
            imports.push(line.to_owned());
        } else if line.starts_with("bail!(") {
            body.push_str("bail!(...)");
        } else {
            body.push_str(
                &line
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect::<String>(),
            );
        }
    }
    imports.sort();
    let mut shape = imports.concat();
    shape.push_str(&body);
    while shape.contains(",)") {
        shape = shape.replace(",)", ")");
    }
    shape
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
    let changed = if relative_path.ends_with(".rs") {
        write_rust_source_if_changed(&path, source)?
    } else {
        write_if_changed(&path, source.as_bytes())?
    };
    if changed && relative_path.ends_with(".rs") {
        result.changed_rust_sources.insert(relative_path.to_owned());
    }
    result.generated_files.push(relative_path.to_owned());
    Ok(())
}

fn write_if_changed(path: &Path, source: &[u8]) -> Result<bool> {
    match fs::read(path) {
        Ok(current) if current == source => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("读取待生成文件失败: {}", path.display()));
        }
    }
    fs::write(path, source).with_context(|| format!("写入生成文件失败: {}", path.display()))?;
    Ok(true)
}

fn remove_stale_generated_pages(
    module_dir: &Path,
    expected: &BTreeSet<String>,
    manifest: &ServiceStubManifest,
) -> Result<()> {
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
        let implementation_relative = format!("src/generated/{module_name}/service_impl.rs");
        let implementation_path = path.join("service_impl.rs");
        let preserve_manual_implementation = !implementation_path.is_file()
            || manifest
                .files
                .get(&implementation_relative)
                .is_none_or(|expected_hash| {
                    fs::read(&implementation_path)
                        .is_ok_and(|source| source_hash(&source) != *expected_hash)
                });
        if preserve_manual_implementation {
            remove_generated_page_except_service_impl(&path)?;
            continue;
        }
        fs::remove_dir_all(path).context("删除失效生成页面目录失败")?;
    }
    Ok(())
}

fn remove_generated_page_except_service_impl(page_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(page_dir).context("读取生成页面目录失败")? {
        let entry = entry.context("读取生成页面文件失败")?;
        if !entry
            .file_type()
            .context("读取生成页面文件类型失败")?
            .is_file()
        {
            continue;
        }
        if entry.file_name() == "service_impl.rs" {
            continue;
        }
        fs::remove_file(entry.path()).context("删除失效生成文件失败")?;
    }
    Ok(())
}

fn remove_stale_generated_page_files(module_dir: &Path, page_dir: &str) -> Result<()> {
    const EXPECTED_FILES: &[&str] = &[
        "controller.rs",
        "mod.rs",
        "model.rs",
        "service.rs",
        "service_impl.rs",
        "util.rs",
    ];

    let page_dir = module_dir.join(page_dir);
    if !page_dir.is_dir() {
        return Ok(());
    }
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
dill.workspace = true
studio = {{ package = "az-studio", path = "../../../app/plugins/studio", default-features = false, features = ["server"] }}

[lints]
workspace = true
"#
    )
}

fn lib_source() -> String {
    r#"#![forbid(unsafe_code)]

mod generated;

pub use generated::{ENDPOINT_COUNT, register};
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
    let endpoint_count = pages.iter().map(|page| page.endpoints.len()).sum::<usize>();
    let _ = writeln!(
        source,
        "\npub const ENDPOINT_COUNT: usize = {endpoint_count};"
    );
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

fn page_mod_source(page: &PageDefinition) -> String {
    format!(
        r#"mod controller;
pub(crate) mod model;
pub(crate) mod service;
mod service_impl;
pub(crate) mod util;

use dill::CatalogBuilder;

pub(crate) fn register(builder: &mut CatalogBuilder) {{
    builder.add::<service_impl::{}>();
    controller::register(builder);
}}
"#,
        format!("{}Impl", service_trait_name(page))
    )
}

fn model_source() -> String {
    r#"pub(crate) type EndpointRequest = studio::ConventionEndpointRequest;
pub(crate) type EndpointFuture<'a> = studio::ConventionEndpointFuture<'a>;
"#
    .to_owned()
}

fn util_source() -> String {
    r#"pub(crate) const fn endpoint_id(value: &'static str) -> &'static str {
    value
}
"#
    .to_owned()
}

fn service_contract_source(page: &PageDefinition) -> String {
    let service_name = service_trait_name(page);
    let methods = endpoint_methods(page);
    let mut source = String::from("use super::model::{EndpointFuture, EndpointRequest};\n\n");
    let _ = writeln!(source, "/// {} 领域服务契约。", page.title);
    let _ = writeln!(source, "pub(crate) trait {service_name}: Send + Sync {{");
    for (method_name, endpoint) in methods {
        let _ = writeln!(source, "    /// {}", endpoint.title);
        let _ = writeln!(source, "    fn {method_name}(");
        source.push_str("        &self,\n");
        source.push_str("        request: EndpointRequest,\n");
        source.push_str("    ) -> EndpointFuture<'_>;\n");
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
use studio::ConventionEndpointProvider;

use super::model::{{EndpointFuture, EndpointRequest}};
use super::service::{service_name};

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
    let endpoint_id = rust_string(&endpoint.id.to_string());

    format!(
        r#"#[dill::component]
#[dill::interface(dyn ConventionEndpointProvider)]
#[dill::scope(dill::Singleton)]
pub(crate) struct {controller_name} {{
    service: Arc<dyn {service_name}>,
}}

impl ConventionEndpointProvider for {controller_name} {{
    fn endpoint_id(&self) -> &'static str {{
        super::util::endpoint_id({endpoint_id})
    }}

    fn handle(&self, request: EndpointRequest) -> EndpointFuture<'_> {{
        self.service.{method_name}(request)
    }}
}}
"#
    )
}

fn service_implementation_source(page: &PageDefinition) -> String {
    let service_name = service_trait_name(page);
    let implementation_name = format!("{service_name}Impl");
    let methods = endpoint_methods(page);
    let mut source = format!(
        "{SERVICE_STUB_COMMENT}use anyhow::bail;\nuse super::model::{{EndpointFuture, EndpointRequest}};\nuse super::service::{service_name};\n\n"
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
        source.push_str("        request: EndpointRequest,\n");
        source.push_str("    ) -> EndpointFuture<'_> {\n");
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
