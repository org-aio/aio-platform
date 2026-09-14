mod git;
mod manifest;
mod packaging;
mod publication;
mod schema;

use std::{collections::HashSet, fs, path::Path};

use anyhow::{Context as _, Result, ensure};
use toml_edit::{DocumentMut, InlineTable, Item, Value};

pub use manifest::PluginSource;
pub use packaging::{PackageOptions, package};
pub(crate) use publication::DEFAULT_PUBLISH_URL;
pub use publication::{PublicationOptions, publish};
pub use schema::write_schemas;

use crate::initialize::template;
use manifest::{
    InstalledPackage, InstalledPlugin, PluginLock, PluginRuntime, ProjectManifest,
    RepositoryPackage, read_lock, read_package_name, read_project, read_repository,
    validate_relative_path, write_lock, write_project,
};

pub fn validate(root: &Path) -> Result<()> {
    let report = az_plugin_manifest::validate_repository(root)?;
    let artifact = report
        .artifact
        .as_deref()
        .map_or_else(|| "-".to_owned(), |path| path.display().to_string());
    println!(
        "插件校验通过: runtime={} subplugins={} pages={} artifact={artifact}",
        runtime_name(report.runtime),
        report.subplugin_count,
        report.page_count,
    );
    Ok(())
}

pub fn install(root: &Path, source: PluginSource) -> Result<()> {
    let mut project = read_project(root)?;
    ensure!(
        !project
            .plugins
            .iter()
            .any(|plugin| plugin.git == source.git),
        "插件来源已安装: {}",
        source.git
    );
    project.plugins.push(source);
    apply(root, &project)?;
    println!("插件已安装并同步");
    Ok(())
}

pub fn uninstall(root: &Path, git_source: &str) -> Result<()> {
    let mut project = read_project(root)?;
    let previous_len = project.plugins.len();
    project.plugins.retain(|plugin| plugin.git != git_source);
    ensure!(
        project.plugins.len() != previous_len,
        "插件来源未安装: {git_source}"
    );
    apply(root, &project)?;
    println!("插件已卸载并同步");
    Ok(())
}

pub fn sync(root: &Path) -> Result<()> {
    let project = read_project(root)?;
    apply(root, &project)?;
    println!("插件已同步");
    Ok(())
}

pub fn list(root: &Path) -> Result<()> {
    let project = read_project(root)?;
    let lock = read_lock(root)?;
    if project.plugins.is_empty() {
        println!("未配置 Git 插件");
        return Ok(());
    }
    for source in project.plugins {
        let installed = lock.plugins.iter().find(|plugin| plugin.git == source.git);
        match installed {
            Some(plugin) => {
                let capabilities = [
                    plugin.client.as_ref().map(|_| "client"),
                    plugin.server.as_ref().map(|_| "server"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(",");
                println!(
                    "{}  {}  {}  {}",
                    capabilities, plugin.revision, plugin.git, plugin.directory
                );
            }
            None => println!("未同步  {}", source.git),
        }
    }
    Ok(())
}

fn apply(root: &Path, project: &ProjectManifest) -> Result<()> {
    ensure!(root.join("Cargo.toml").is_file(), "当前目录不是 Cargo 项目");
    let previous_lock = read_lock(root)?;
    let installed = resolve_plugins(root, &project.plugins)?;
    update_cargo_manifest(root, &previous_lock.plugins, &installed)?;
    update_plugin_source(root, &installed)?;
    write_project(root, project)?;
    write_lock(
        root,
        &PluginLock {
            plugins: installed.clone(),
        },
    )?;
    remove_stale_checkouts(root, &previous_lock.plugins, &installed)?;
    Ok(())
}

fn resolve_plugins(root: &Path, sources: &[PluginSource]) -> Result<Vec<InstalledPlugin>> {
    let mut dependencies = HashSet::new();
    let mut directories = HashSet::new();
    let mut installed = Vec::with_capacity(sources.len());
    for source in sources {
        ensure!(!source.git.trim().is_empty(), "插件 Git 地址不能为空");
        let directory = repository_directory(&source.git);
        ensure!(
            directories.insert(directory.clone()),
            "插件缓存目录冲突: {directory}"
        );
        let checkout = root.join(".aio/plugins").join(&directory);
        let revision = git::checkout(&source.git, source.rev.as_deref(), &checkout)
            .with_context(|| format!("拉取插件失败: {}", source.git))?;
        let repository = read_repository(&checkout)
            .with_context(|| format!("发现插件清单失败: {}", source.git))?;
        ensure!(
            repository.plugin.runtime.is_none(),
            "在线运行时插件不能装配到 Rust 源码宿主，请使用 aio plugin validate: {}",
            source.git
        );
        ensure!(
            repository.plugin.client.is_some() || repository.plugin.server.is_some(),
            "插件必须至少声明 client 或 server 能力: {}",
            source.git
        );
        let client = resolve_package(
            &checkout,
            repository.plugin.client.as_ref(),
            &mut dependencies,
        )?;
        let server = resolve_package(
            &checkout,
            repository.plugin.server.as_ref(),
            &mut dependencies,
        )?;
        installed.push(InstalledPlugin {
            git: source.git.clone(),
            revision: revision.trim().to_owned(),
            directory,
            client,
            server,
        });
    }
    Ok(installed)
}

fn update_cargo_manifest(
    root: &Path,
    previous: &[InstalledPlugin],
    installed: &[InstalledPlugin],
) -> Result<()> {
    let path = root.join("Cargo.toml");
    let text = fs::read_to_string(&path)
        .with_context(|| format!("读取 Cargo.toml 失败: {}", path.display()))?;
    let mut document = text
        .parse::<DocumentMut>()
        .with_context(|| format!("解析 Cargo.toml 失败: {}", path.display()))?;
    let dependencies = document["dependencies"]
        .as_table_mut()
        .context("Cargo.toml 缺少 [dependencies]")?;

    for package in previous.iter().flat_map(plugin_packages) {
        dependencies.remove(&package.dependency);
    }
    for plugin in installed {
        for package in plugin_packages(plugin) {
            ensure!(
                !dependencies.contains_key(&package.dependency),
                "Cargo 依赖名称已被占用: {}",
                package.dependency
            );
            let mut dependency = InlineTable::new();
            dependency.insert("package", Value::from(package.package.clone()));
            dependency.insert("git", Value::from(cargo_git_source(&plugin.git)?));
            dependency.insert("rev", Value::from(plugin.revision.clone()));
            dependency.insert("optional", Value::from(true));
            dependencies.insert(
                &package.dependency,
                Item::Value(Value::InlineTable(dependency)),
            );
        }
    }
    update_feature_dependencies(&mut document, previous, installed)?;
    fs::write(&path, document.to_string())
        .with_context(|| format!("更新 Cargo.toml 失败: {}", path.display()))
}

fn update_plugin_source(root: &Path, installed: &[InstalledPlugin]) -> Result<()> {
    let clients = installed
        .iter()
        .filter_map(|plugin| plugin.client.as_ref())
        .map(|package| package.dependency.clone())
        .collect::<Vec<_>>();
    let servers = installed
        .iter()
        .filter_map(|plugin| plugin.server.as_ref())
        .map(|package| package.dependency.clone())
        .collect::<Vec<_>>();
    let path = root.join("src/plugins.rs");
    fs::write(&path, template::plugins_source(&clients, &servers))
        .with_context(|| format!("生成插件注册入口失败: {}", path.display()))
}

fn remove_stale_checkouts(
    root: &Path,
    previous: &[InstalledPlugin],
    installed: &[InstalledPlugin],
) -> Result<()> {
    let active = installed
        .iter()
        .map(|plugin| plugin.directory.as_str())
        .collect::<HashSet<_>>();
    for plugin in previous {
        if active.contains(plugin.directory.as_str()) {
            continue;
        }
        validate_cache_directory(&plugin.directory)?;
        let path = root.join(".aio/plugins").join(&plugin.directory);
        if path.exists() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("删除插件缓存失败: {}", path.display()))?;
        }
    }
    Ok(())
}

fn repository_directory(source: &str) -> String {
    let basename = source
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("plugin")
        .trim_end_matches(".git");
    let slug = basename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "plugin" } else { slug };
    format!("{slug}-{:08x}", source_hash(source))
}

fn source_hash(source: &str) -> u32 {
    source.as_bytes().iter().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

fn rust_crate_name(package: &str) -> Result<String> {
    let dependency = package.replace('-', "_");
    let mut characters = dependency.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_');
    let valid_rest =
        characters.all(|character| character.is_ascii_alphanumeric() || character == '_');
    ensure!(
        valid_first && valid_rest && !reserved_identifier(&dependency),
        "Cargo 包名不能作为 Rust crate 使用: {package}"
    );
    Ok(dependency)
}

fn reserved_identifier(value: &str) -> bool {
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
    )
}

fn resolve_package(
    checkout: &Path,
    package: Option<&RepositoryPackage>,
    dependencies: &mut HashSet<String>,
) -> Result<Option<InstalledPackage>> {
    let Some(package) = package else {
        return Ok(None);
    };
    ensure!(
        package.runtime == PluginRuntime::RustSource,
        "当前宿主尚不支持 {:?} 插件运行时",
        package.runtime
    );
    validate_relative_path(&package.path)?;
    let package_name = read_package_name(&checkout.join(&package.path))?;
    let dependency = rust_crate_name(&package_name)?;
    ensure!(
        dependencies.insert(dependency.clone()),
        "插件 Cargo crate 名称冲突: {dependency}"
    );
    Ok(Some(InstalledPackage {
        package: package_name,
        dependency,
        package_path: package.path.clone(),
    }))
}

fn plugin_packages(plugin: &InstalledPlugin) -> impl Iterator<Item = &InstalledPackage> {
    plugin.client.iter().chain(plugin.server.iter())
}

fn update_feature_dependencies(
    document: &mut DocumentMut,
    previous: &[InstalledPlugin],
    installed: &[InstalledPlugin],
) -> Result<()> {
    let previous_clients = previous
        .iter()
        .filter_map(|plugin| plugin.client.as_ref())
        .map(|package| format!("dep:{}", package.dependency))
        .collect::<HashSet<_>>();
    let previous_servers = previous
        .iter()
        .filter_map(|plugin| plugin.server.as_ref())
        .map(|package| format!("dep:{}", package.dependency))
        .collect::<HashSet<_>>();
    let clients = installed
        .iter()
        .filter_map(|plugin| plugin.client.as_ref())
        .map(|package| format!("dep:{}", package.dependency))
        .collect::<Vec<_>>();
    let servers = installed
        .iter()
        .filter_map(|plugin| plugin.server.as_ref())
        .map(|package| format!("dep:{}", package.dependency))
        .collect::<Vec<_>>();

    update_feature(document, "web", &previous_clients, &clients)?;
    update_feature(document, "desktop", &previous_clients, &clients)?;
    update_feature(document, "server", &previous_servers, &servers)
}

fn update_feature(
    document: &mut DocumentMut,
    feature: &str,
    previous: &HashSet<String>,
    installed: &[String],
) -> Result<()> {
    let values = document["features"][feature]
        .as_array_mut()
        .with_context(|| format!("Cargo.toml 缺少 [features].{feature} 数组"))?;
    let mut index = values.len();
    while index > 0 {
        index -= 1;
        if values
            .get(index)
            .and_then(Value::as_str)
            .is_some_and(|value| previous.contains(value))
        {
            values.remove(index);
        }
    }
    for dependency in installed {
        values.push(dependency.as_str());
    }
    Ok(())
}

fn cargo_git_source(source: &str) -> Result<String> {
    let path = Path::new(source);
    if !path.exists() {
        return Ok(source.to_owned());
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("解析本地插件 Git 路径失败: {}", path.display()))?;
    Ok(format!("file://{}", canonical.to_string_lossy()))
}

fn validate_cache_directory(directory: &str) -> Result<()> {
    ensure!(
        Path::new(directory).components().count() == 1
            && Path::new(directory)
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "插件锁定文件包含无效缓存目录: {directory}"
    );
    Ok(())
}

fn runtime_name(runtime: PluginRuntime) -> &'static str {
    match runtime {
        PluginRuntime::RustSource => "rust-source",
        PluginRuntime::PageDefinition => "page-definition",
        PluginRuntime::WasmComponent => "wasm-component",
        PluginRuntime::Process => "process",
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;
    use crate::initialize::{self, ApplicationOptions};

    #[test]
    fn installs_and_uninstalls_git_repository_plugin() -> Result<()> {
        let workspace = tempdir()?;
        let application = workspace.path().join("sample-app");
        initialize::application(ApplicationOptions {
            path: application.clone(),
            name: None,
            title: None,
            network: initialize::NetworkProfile::default(),
        })?;
        let plugin = create_plugin_repository(workspace.path())?;
        let source = PluginSource {
            git: plugin.to_string_lossy().into_owned(),
            rev: None,
        };

        install(&application, source.clone())?;

        let cargo = fs::read_to_string(application.join("Cargo.toml"))?;
        let registrations = fs::read_to_string(application.join("src/plugins.rs"))?;
        assert!(cargo.contains("sample_pages_client"));
        assert!(cargo.contains("sample_pages_server"));
        assert!(registrations.contains("sample_pages_client::register"));
        assert!(registrations.contains("sample_pages_server::register(&mut builder)?"));
        assert_eq!(read_project(&application)?.plugins, vec![source.clone()]);
        let lock = read_lock(&application)?;
        assert_eq!(lock.plugins.len(), 1);
        assert!(lock.plugins[0].client.is_some());
        assert!(lock.plugins[0].server.is_some());

        uninstall(&application, &source.git)?;

        let cargo = fs::read_to_string(application.join("Cargo.toml"))?;
        assert!(!cargo.contains("sample_pages_client"));
        assert!(!cargo.contains("sample_pages_server"));
        assert!(read_project(&application)?.plugins.is_empty());
        assert!(read_lock(&application)?.plugins.is_empty());
        Ok(())
    }

    fn create_plugin_repository(root: &Path) -> Result<PathBuf> {
        let path = root.join("sample-pages");
        fs::create_dir_all(path.join("client/src"))?;
        fs::create_dir_all(path.join("server/src"))?;
        fs::write(
            path.join("aio-plugin.toml"),
            "[plugin.client]\npath = \"client\"\n\n[plugin.server]\npath = \"server\"\n",
        )?;
        fs::write(
            path.join("Cargo.toml"),
            "[workspace]\nmembers = [\"client\", \"server\"]\nresolver = \"2\"\n",
        )?;
        fs::write(
            path.join("client/Cargo.toml"),
            "[package]\nname = \"sample-pages-client\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        fs::write(
            path.join("server/Cargo.toml"),
            "[package]\nname = \"sample-pages-server\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        fs::write(path.join("client/src/lib.rs"), "pub fn register() {}\n")?;
        fs::write(path.join("server/src/lib.rs"), "pub fn register() {}\n")?;
        run_git(&path, ["init", "--quiet"])?;
        run_git(&path, ["config", "user.email", "test@example.com"])?;
        run_git(&path, ["config", "user.name", "AIO Test"])?;
        run_git(&path, ["add", "."])?;
        run_git(&path, ["commit", "--quiet", "-m", "init"])?;
        Ok(path)
    }

    fn run_git<'a>(path: &Path, arguments: impl IntoIterator<Item = &'a str>) -> Result<()> {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(path)
            .status()?;
        ensure!(status.success(), "测试 Git 命令失败");
        Ok(())
    }

    #[test]
    fn repository_directory_is_stable_and_source_specific() {
        assert_eq!(
            repository_directory("https://example.com/team/orders.git"),
            repository_directory("https://example.com/team/orders.git")
        );
        assert_ne!(
            repository_directory("https://example.com/team/orders.git"),
            repository_directory("https://another.example/team/orders.git")
        );
    }
}
