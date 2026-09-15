use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Component as PathComponent, Path, PathBuf},
};

use crate::{
    MarketplaceManifest, PageBody, PageDefinition, PluginRuntime, RepositoryManifest,
    RepositoryPackage, validate_wasm_component,
};
use anyhow::{Context as _, Result, bail, ensure};

const MAX_PAGE_STATE_KEYS: usize = 64;
const MAX_PAGE_STATE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    pub runtime: PluginRuntime,
    pub artifact: Option<PathBuf>,
    pub page_count: usize,
    pub subplugin_count: usize,
}

pub fn read_manifest(root: &Path) -> Result<RepositoryManifest> {
    let path = root.join("aio-plugin.toml");
    let source = fs::read_to_string(&path)
        .with_context(|| format!("读取插件清单失败: {}", path.display()))?;
    parse_manifest(&source)
}

pub fn parse_manifest(source: &str) -> Result<RepositoryManifest> {
    let manifest = toml::from_str::<RepositoryManifest>(source).context("解析插件清单失败")?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &RepositoryManifest) -> Result<()> {
    let plugin = &manifest.plugin;
    crate::validate_dependencies(&plugin.dependencies)?;
    ensure!(
        plugin.runtime.is_some() || plugin.client.is_some() || plugin.server.is_some(),
        "插件必须声明 runtime、client 或 server 能力"
    );
    for package in [plugin.client.as_ref(), plugin.server.as_ref()]
        .into_iter()
        .flatten()
    {
        validate_source_package(package)?;
    }
    if let Some(frontend) = &plugin.frontend {
        crate::validate_frontend_path(&frontend.path)?;
        ensure!(plugin.runtime.is_some(), "前端二进制必须与运行产物共同声明");
        ensure!(
            plugin.client.is_none() && plugin.server.is_none(),
            "前端二进制不能与源码装配混用"
        );
    }
    if let Some(runtime) = &plugin.runtime {
        ensure!(
            runtime.kind != PluginRuntime::RustSource,
            "rust-source 必须通过 plugin.client 或 plugin.server 声明"
        );
        validate_relative_path(&runtime.artifact, "插件 artifact")?;
        if let Some(requirement) = &runtime.host_version {
            validate_name(requirement, "宿主版本约束")?;
            semver::VersionReq::parse(requirement)
                .with_context(|| format!("宿主版本约束无效: {requirement}"))?;
        }
        validate_runtime_options(runtime)?;
    }
    if let Some(marketplace) = &plugin.marketplace {
        validate_marketplace(marketplace)?;
    }
    validate_capabilities(&plugin.capabilities.network, "网络能力")?;
    validate_capabilities(&plugin.capabilities.filesystem, "文件系统能力")?;
    validate_subplugins(manifest)
}

fn validate_marketplace(marketplace: &MarketplaceManifest) -> Result<()> {
    validate_name(&marketplace.title, "市场标题")?;
    validate_name(&marketplace.summary, "市场简介")?;
    validate_name(&marketplace.license, "市场许可证")?;
    ensure!(!marketplace.tags.is_empty(), "市场标签不能为空");
    for tag in &marketplace.tags {
        validate_name(tag, "市场标签")?;
    }
    Ok(())
}

pub fn validate_host_compatibility(
    manifest: &RepositoryManifest,
    host_version: &str,
) -> Result<()> {
    let host_version = semver::Version::parse(host_version)
        .with_context(|| format!("宿主版本无效: {host_version}"))?;
    let Some(requirement) = manifest
        .plugin
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.host_version.as_deref())
    else {
        return Ok(());
    };
    let requirement = semver::VersionReq::parse(requirement)
        .with_context(|| format!("宿主版本约束无效: {requirement}"))?;
    ensure!(
        requirement.matches(&host_version),
        "插件需要宿主版本 {requirement}，当前为 {host_version}"
    );
    Ok(())
}

fn validate_runtime_options(runtime: &crate::RuntimeManifest) -> Result<()> {
    if runtime.kind == PluginRuntime::Process {
        let image = runtime
            .container_image
            .as_deref()
            .context("process 插件必须声明 container_image")?;
        validate_container_image(image)?;
        ensure!(
            !runtime.entrypoint.is_empty(),
            "process 插件必须声明 entrypoint"
        );
        for argument in &runtime.entrypoint {
            validate_name(argument, "process 启动参数")?;
            ensure!(!argument.contains('\0'), "process 启动参数不能包含 NUL");
        }
        ensure!(
            runtime
                .entrypoint
                .iter()
                .filter(|argument| argument.as_str() == "{artifact}")
                .count()
                == 1,
            "process entrypoint 必须且只能引用一次 {{artifact}}"
        );
        let health_check = runtime
            .health_check
            .as_deref()
            .context("process 插件必须声明 health_check")?;
        ensure!(
            health_check.starts_with('/')
                && !health_check.contains('?')
                && !health_check.contains('#'),
            "process health_check 必须是不含查询或片段的绝对路径"
        );
        if let Some(timeout) = runtime.shutdown_timeout_seconds {
            ensure!(
                (1..=60).contains(&timeout),
                "process shutdown_timeout_seconds 必须在 1..=60"
            );
        }
        if let Some(size) = runtime.temporary_storage_mb {
            ensure!(
                (16..=128).contains(&size),
                "process temporary_storage_mb 必须在 16..=128"
            );
        }
        return Ok(());
    }
    ensure!(
        runtime.container_image.is_none()
            && runtime.entrypoint.is_empty()
            && runtime.health_check.is_none()
            && runtime.shutdown_timeout_seconds.is_none()
            && runtime.temporary_storage_mb.is_none(),
        "container_image、entrypoint、health_check、shutdown_timeout_seconds 和 temporary_storage_mb 只能用于 process 插件"
    );
    Ok(())
}

fn validate_container_image(image: &str) -> Result<()> {
    validate_name(image, "process 容器镜像")?;
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        bail!("process container_image 必须锁定 sha256 digest");
    };
    ensure!(
        !repository.is_empty()
            && !repository.contains(char::is_whitespace)
            && digest.len() == 64
            && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "process container_image 必须是有效的镜像@sha256:digest"
    );
    Ok(())
}

pub fn validate_repository(root: &Path) -> Result<ValidationReport> {
    let manifest = read_manifest(root)?;
    for package in [
        manifest.plugin.client.as_ref(),
        manifest.plugin.server.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        source_package_path(root, package)?;
    }
    let Some(runtime) = &manifest.plugin.runtime else {
        return Ok(ValidationReport {
            runtime: PluginRuntime::RustSource,
            artifact: None,
            page_count: 0,
            subplugin_count: manifest.plugin.subplugins.len(),
        });
    };
    let artifact = artifact_path(root, &runtime.artifact)?;
    let frontend_files = crate::frontend_files(root, &manifest)?;
    let page_count = match runtime.kind {
        PluginRuntime::PageDefinition => {
            let pages = crate::parse_page_definitions(&fs::read(&artifact).with_context(|| {
                format!("读取 PageDefinition 产物失败: {}", artifact.display())
            })?)
            .context("解析 PageDefinition 产物失败")?;
            ensure!(
                !pages.is_empty(),
                "page-definition 插件至少需要贡献一个页面"
            );
            validate_page_definitions(&pages)?;
            ensure!(
                pages
                    .iter()
                    .all(|page| !matches!(page.body, PageBody::Actions { .. })),
                "page-definition 插件不能声明需要运行时处理的页面动作"
            );
            validate_declared_pages(&manifest, &pages)?;
            crate::validate_frontend_pages(
                &manifest,
                &pages,
                frontend_files.keys().map(String::as_str),
            )?;
            pages.len()
        }
        PluginRuntime::WasmComponent => {
            let pages = validate_wasm_component(&fs::read(&artifact).with_context(|| {
                format!("读取 Wasm Component 产物失败: {}", artifact.display())
            })?)?;
            validate_declared_pages(&manifest, &pages)?;
            crate::validate_frontend_pages(
                &manifest,
                &pages,
                frontend_files.keys().map(String::as_str),
            )?;
            pages.len()
        }
        PluginRuntime::Process => 0,
        PluginRuntime::RustSource => bail!("rust-source 不使用 plugin.runtime"),
    };
    Ok(ValidationReport {
        runtime: runtime.kind,
        artifact: Some(artifact),
        page_count,
        subplugin_count: manifest.plugin.subplugins.len(),
    })
}

fn source_package_path(root: &Path, package: &RepositoryPackage) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("解析插件仓库路径失败: {}", root.display()))?;
    let package_root = root.join(&package.path);
    ensure!(
        package_root.is_dir(),
        "Rust 插件包目录不存在: {}",
        package.path
    );
    let package_root = package_root
        .canonicalize()
        .with_context(|| format!("解析 Rust 插件包路径失败: {}", package_root.display()))?;
    ensure!(
        package_root.starts_with(&root),
        "Rust 插件包不能通过符号链接离开仓库: {}",
        package.path
    );
    ensure!(
        package_root.join("Cargo.toml").is_file(),
        "Rust 插件包缺少 Cargo.toml: {}",
        package.path
    );
    Ok(package_root)
}

pub fn artifact_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative, "插件 artifact")?;
    let root = root
        .canonicalize()
        .with_context(|| format!("解析插件仓库路径失败: {}", root.display()))?;
    let artifact = root.join(relative);
    ensure!(artifact.is_file(), "插件 artifact 不存在: {relative}");
    let artifact = artifact
        .canonicalize()
        .with_context(|| format!("解析插件 artifact 路径失败: {}", artifact.display()))?;
    ensure!(
        artifact.starts_with(&root),
        "插件 artifact 不能通过符号链接离开仓库: {relative}"
    );
    Ok(artifact)
}

pub fn validate_page_definitions(pages: &[PageDefinition]) -> Result<()> {
    let mut ids = HashSet::new();
    let mut scenes = HashMap::<String, String>::new();
    let mut groups = HashMap::<String, (String, Option<String>, String, Option<String>)>::new();
    for page in pages {
        validate_name(&page.id, "页面 id")?;
        validate_name(&page.label, "页面标题")?;
        validate_name(&page.scene.id, "页面场景 id")?;
        validate_name(&page.scene.label, "页面场景标题")?;
        if let Some(label) = scenes.insert(page.scene.id.clone(), page.scene.label.clone()) {
            ensure!(
                label == page.scene.label,
                "同一场景 id 的标题不一致: {}",
                page.scene.id
            );
        }
        ensure!(
            ids.insert(page.id.as_str()),
            "插件页面 id 重复: {}",
            page.id
        );
        if let Some(icon) = &page.icon {
            validate_name(icon, "页面图标")?;
        }
        if let Some(permission) = &page.required_permission {
            validate_name(permission, "页面权限")?;
        }
        let mut path_ids = HashSet::new();
        let mut parent = None::<String>;
        for group in &page.menu_path {
            validate_name(&group.id, "菜单分组 id")?;
            validate_name(&group.label, "菜单分组标题")?;
            if let Some(icon) = &group.icon {
                validate_name(icon, "菜单分组图标")?;
            }
            ensure!(
                path_ids.insert(group.id.as_str()),
                "菜单分组路径形成循环: {} -> {}",
                page.id,
                group.id
            );
            let location = (
                page.scene.id.clone(),
                parent.clone(),
                group.label.clone(),
                group.icon.clone(),
            );
            if let Some(existing) = groups.get(&group.id) {
                ensure!(
                    existing == &location,
                    "同一菜单分组 id 的场景、父节点或展示信息不一致: {}",
                    group.id
                );
            } else {
                groups.insert(group.id.clone(), location);
            }
            parent = Some(group.id.clone());
        }
        match &page.body {
            PageBody::Frontend { entry } => {
                crate::validate_frontend_path(entry)?;
                ensure!(entry.ends_with(".html"), "前端页面入口必须是 HTML 文档");
            }
            PageBody::Counter { title, button } => {
                validate_name(title, "计数页标题")?;
                validate_name(button, "计数页按钮")?;
            }
            PageBody::Text { title, content } => {
                validate_name(title, "文本页标题")?;
                validate_name(content, "文本页内容")?;
            }
            PageBody::Actions {
                title,
                content,
                state,
                actions,
            } => {
                validate_name(title, "动作页标题")?;
                validate_name(content, "动作页内容")?;
                ensure!(
                    state.len() <= MAX_PAGE_STATE_KEYS,
                    "动作页状态字段不能超过 {MAX_PAGE_STATE_KEYS} 个"
                );
                for key in state.keys() {
                    validate_name(key, "动作页状态字段")?;
                }
                ensure!(
                    serde_json::to_vec(state)?.len() <= MAX_PAGE_STATE_BYTES,
                    "动作页状态不能超过 {MAX_PAGE_STATE_BYTES} 字节"
                );
                ensure!(!actions.is_empty(), "动作页至少需要一个动作");
                let mut ids = HashSet::new();
                for action in actions {
                    validate_name(&action.id, "页面动作 id")?;
                    validate_name(&action.label, "页面动作标题")?;
                    ensure!(
                        ids.insert(action.id.as_str()),
                        "页面动作 id 重复: {}",
                        action.id
                    );
                }
            }
        }
    }
    for group in groups.keys() {
        ensure!(
            !ids.contains(group.as_str()),
            "菜单分组 id 与页面 id 冲突: {group}"
        );
    }
    Ok(())
}

fn validate_source_package(package: &RepositoryPackage) -> Result<()> {
    ensure!(
        package.runtime == PluginRuntime::RustSource,
        "plugin.client 和 plugin.server 只能使用 rust-source"
    );
    validate_relative_package_path(&package.path)
}

fn validate_relative_package_path(relative: &str) -> Result<()> {
    ensure!(!relative.trim().is_empty(), "Rust 插件包路径不能为空");
    let path = Path::new(relative);
    ensure!(
        !path.is_absolute(),
        "Rust 插件包路径不能是绝对路径: {relative}"
    );
    ensure!(
        path.components()
            .all(|component| matches!(component, PathComponent::CurDir | PathComponent::Normal(_))),
        "Rust 插件包路径不能离开仓库: {relative}"
    );
    Ok(())
}

fn validate_relative_path(relative: &str, label: &str) -> Result<()> {
    ensure!(!relative.trim().is_empty(), "{label} 不能为空");
    ensure!(
        !relative.contains(['\\', ':', '\0'])
            && relative
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "{label} 必须使用规范化的跨平台相对路径: {relative}"
    );
    let path = Path::new(relative);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, PathComponent::Normal(_))),
        "{label} 必须是仓库内相对路径: {relative}"
    );
    Ok(())
}

fn validate_capabilities(values: &[String], label: &str) -> Result<()> {
    let mut unique = HashSet::new();
    for value in values {
        validate_name(value, label)?;
        ensure!(unique.insert(value.as_str()), "{label}重复: {value}");
    }
    Ok(())
}

fn validate_subplugins(manifest: &RepositoryManifest) -> Result<()> {
    let plugins = manifest
        .plugin
        .subplugins
        .iter()
        .map(|plugin| (plugin.id.as_str(), plugin))
        .collect::<HashMap<_, _>>();
    ensure!(
        plugins.len() == manifest.plugin.subplugins.len(),
        "子插件 id 重复"
    );
    let mut pages = HashSet::new();
    let mut routes = HashSet::new();
    let mut account_actions = HashSet::new();
    for plugin in &manifest.plugin.subplugins {
        validate_name(&plugin.id, "子插件 id")?;
        validate_contributions(&plugin.pages, "页面", &mut pages)?;
        validate_contributions(&plugin.routes, "路由", &mut routes)?;
        validate_contributions(&plugin.account_actions, "账户动作", &mut account_actions)?;
        let mut dependencies = HashSet::new();
        for dependency in &plugin.dependencies {
            validate_name(dependency, "子插件依赖")?;
            ensure!(
                dependencies.insert(dependency.as_str()),
                "子插件 {} 重复依赖: {dependency}",
                plugin.id
            );
            ensure!(
                plugins.contains_key(dependency.as_str()),
                "子插件 {} 依赖不存在: {dependency}",
                plugin.id
            );
        }
    }
    let mut states = HashMap::new();
    for id in plugins.keys() {
        visit(id, &plugins, &mut states)?;
    }
    Ok(())
}

fn validate_contributions<'a>(
    values: &'a [String],
    label: &str,
    unique: &mut HashSet<&'a str>,
) -> Result<()> {
    for value in values {
        validate_name(value, label)?;
        ensure!(
            unique.insert(value.as_str()),
            "子插件{label}声明重复: {value}"
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Visited,
}

fn visit<'a>(
    id: &'a str,
    plugins: &HashMap<&'a str, &'a crate::SubpluginManifest>,
    states: &mut HashMap<&'a str, VisitState>,
) -> Result<()> {
    match states.get(id) {
        Some(VisitState::Visiting) => bail!("子插件依赖存在循环: {id}"),
        Some(VisitState::Visited) => return Ok(()),
        None => {}
    }
    states.insert(id, VisitState::Visiting);
    for dependency in &plugins[id].dependencies {
        visit(dependency, plugins, states)?;
    }
    states.insert(id, VisitState::Visited);
    Ok(())
}

fn declared_pages(manifest: &RepositoryManifest) -> BTreeSet<&str> {
    manifest
        .plugin
        .subplugins
        .iter()
        .flat_map(|plugin| plugin.pages.iter().map(String::as_str))
        .collect()
}

pub fn validate_declared_pages(
    manifest: &RepositoryManifest,
    pages: &[PageDefinition],
) -> Result<()> {
    ensure!(
        manifest.plugin.frontend.is_some()
            || pages
                .iter()
                .all(|page| !matches!(page.body, PageBody::Frontend { .. })),
        "前端页面必须声明 plugin.frontend"
    );
    let declared = declared_pages(manifest);
    let actual = pages
        .iter()
        .map(|page| page.id.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        declared == actual,
        "子插件页面声明与 PageDefinition 不一致: 声明={declared:?}, 产物={actual:?}"
    );
    validate_declared_account_actions(manifest, &actual)?;
    Ok(())
}

fn validate_declared_account_actions(
    manifest: &RepositoryManifest,
    pages: &BTreeSet<&str>,
) -> Result<()> {
    for action in manifest
        .plugin
        .subplugins
        .iter()
        .flat_map(|plugin| plugin.account_actions.iter())
    {
        ensure!(
            pages.contains(action.as_str()),
            "账户动作必须指向同一插件已声明的页面: {action}"
        );
    }
    Ok(())
}

fn validate_name(value: &str, label: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{label} 不能为空");
    ensure!(value.trim() == value, "{label} 不能包含首尾空白: {value:?}");
    Ok(())
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
