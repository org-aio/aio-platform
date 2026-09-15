mod fullstack;
mod kotlin;
mod language;
mod network;
mod scaffold;
pub(crate) mod template;
mod typescript;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail, ensure};
use az_plugin_manifest::PluginRuntime;

pub use language::{PluginLanguage, PluginTemplate, WebFramework, parse_runtime};
pub use network::NetworkProfile;

pub struct ApplicationOptions {
    pub path: PathBuf,
    pub name: Option<String>,
    pub title: Option<String>,
    pub network: NetworkProfile,
}

#[derive(Debug)]
pub struct RepositoryPluginOptions {
    pub path: PathBuf,
    pub name: Option<String>,
    pub title: Option<String>,
    pub template: PluginTemplate,
    pub network: NetworkProfile,
}

pub fn application(options: ApplicationOptions) -> Result<()> {
    let name = resolve_package_name(&options.path, options.name)?;
    let title = options.title.unwrap_or_else(|| name.clone());
    prepare_directory(&options.path)?;
    create_directory(&options.path.join("src"))?;
    write(
        &options.path.join("Cargo.toml"),
        &template::application_cargo(&name),
    )?;
    write(
        &options.path.join("Dioxus.toml"),
        &template::dioxus_manifest(&name, &title),
    )?;
    write(&options.path.join("index.html"), template::INDEX_HTML)?;
    write(
        &options.path.join("rust-toolchain.toml"),
        template::RUST_TOOLCHAIN,
    )?;
    write(
        &options.path.join("aio.toml"),
        &template::aio_manifest(&name, &title),
    )?;
    write(&options.path.join(".gitignore"), template::GITIGNORE)?;
    write(
        &options.path.join("README.md"),
        &template::application_readme(&title),
    )?;
    write(
        &options.path.join("src/main.rs"),
        &template::application_main(&title),
    )?;
    write(
        &options.path.join("src/plugins.rs"),
        &template::plugins_source(&[], &[]),
    )?;
    write(
        &options.path.join("src/server.rs"),
        &template::application_server(&name),
    )?;
    write(
        &options.path.join("Dockerfile"),
        &template::dockerfile(&name),
    )?;
    write(
        &options.path.join("compose.yaml"),
        template::COMPOSE_MANIFEST,
    )?;
    network::configure(&options.path, PluginLanguage::Rust, options.network, false)?;
    println!("已初始化应用: {}", options.path.display());
    Ok(())
}

pub fn repository_plugin(options: RepositoryPluginOptions) -> Result<()> {
    let name = resolve_package_name(&options.path, options.name)?;
    let title = options.title.unwrap_or_else(|| name.clone());
    let template = options.template;
    let client_name = format!("{name}-client");
    let server_name = format!("{name}-server");
    prepare_directory(&options.path)?;
    match template {
        PluginTemplate::Fullstack(language) => {
            fullstack::materialize(&options.path, language, &name, &title)?
        }
        PluginTemplate::WebFullstack(framework) => {
            fullstack::materialize_web(&options.path, framework, &name, &title)?
        }
        PluginTemplate::Rust => {
            rust_repository_plugin(&options.path, &name, &title, &client_name, &server_name)?;
        }
        PluginTemplate::KotlinPages => {
            kotlin::repository_plugin(&options.path, &name, &title, PluginRuntime::PageDefinition)?;
        }
        PluginTemplate::KotlinComponent => {
            kotlin::repository_plugin(&options.path, &name, &title, PluginRuntime::WasmComponent)?;
        }
        PluginTemplate::KotlinService => {
            kotlin::repository_plugin(&options.path, &name, &title, PluginRuntime::Process)?;
        }
        PluginTemplate::TypeScriptPages => {
            typescript::repository_plugin(
                &options.path,
                &name,
                &title,
                PluginRuntime::PageDefinition,
            )?;
        }
        PluginTemplate::TypeScriptComponent => {
            typescript::repository_plugin(
                &options.path,
                &name,
                &title,
                PluginRuntime::WasmComponent,
            )?;
        }
        PluginTemplate::TypeScriptService => {
            typescript::repository_plugin(&options.path, &name, &title, PluginRuntime::Process)?;
        }
    }
    let language = match template {
        PluginTemplate::Fullstack(language) => language,
        PluginTemplate::Rust => PluginLanguage::Rust,
        PluginTemplate::KotlinPages
        | PluginTemplate::KotlinComponent
        | PluginTemplate::KotlinService => PluginLanguage::Kotlin,
        _ => PluginLanguage::TypeScript,
    };
    network::configure(
        &options.path,
        language,
        options.network,
        matches!(template, PluginTemplate::Fullstack(PluginLanguage::Kotlin)),
    )?;
    println!(
        "已初始化插件: {} ({})",
        options.path.display(),
        template.label()
    );
    Ok(())
}

fn rust_repository_plugin(
    path: &Path,
    name: &str,
    title: &str,
    client_name: &str,
    server_name: &str,
) -> Result<()> {
    create_directory(&path.join("client/src"))?;
    create_directory(&path.join("server/src"))?;
    write(
        &path.join("Cargo.toml"),
        &template::repository_workspace(client_name, server_name),
    )?;
    write(
        &path.join("client/Cargo.toml"),
        &template::repository_client_cargo(client_name),
    )?;
    write(
        &path.join("server/Cargo.toml"),
        &template::repository_server_cargo(server_name),
    )?;
    write(&path.join("rust-toolchain.toml"), template::RUST_TOOLCHAIN)?;
    write(
        &path.join("aio-plugin.toml"),
        template::repository_manifest(),
    )?;
    write(&path.join(".gitignore"), "/target\n")?;
    write(
        &path.join("README.md"),
        &template::repository_plugin_readme(title),
    )?;
    write(
        &path.join("client/README.md"),
        template::repository_client_readme(),
    )?;
    write(
        &path.join("client/src/lib.rs"),
        &template::repository_client_source(title),
    )?;
    write(
        &path.join("server/README.md"),
        template::repository_server_readme(),
    )?;
    write(
        &path.join("server/src/lib.rs"),
        &template::repository_server_source(name),
    )?;
    Ok(())
}

fn prepare_directory(path: &Path) -> Result<()> {
    if path.exists() {
        ensure!(path.is_dir(), "初始化路径不是目录: {}", path.display());
        let mut entries = fs::read_dir(path)
            .with_context(|| format!("读取初始化目录失败: {}", path.display()))?;
        ensure!(
            entries.next().is_none(),
            "初始化目录必须为空: {}",
            path.display()
        );
        return Ok(());
    }
    create_directory(path)
}

fn create_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("创建目录失败: {}", path.display()))
}

fn write(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("写入文件失败: {}", path.display()))
}

fn resolve_package_name(path: &Path, configured: Option<String>) -> Result<String> {
    let name = configured.or_else(|| {
        path.file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned)
    });
    let Some(name) = name else {
        bail!("无法从目录推导包名，请传入 --name");
    };
    validate_package_name(&name)?;
    Ok(name)
}

fn validate_package_name(name: &str) -> Result<()> {
    let mut characters = name.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase());
    let valid_rest = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    ensure!(
        valid_first && valid_rest,
        "包名必须以小写字母开头，且只包含小写字母、数字和连字符: {name}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn initializes_shell_without_builtin_pages() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("demo-app");

        application(ApplicationOptions {
            path: path.clone(),
            name: None,
            title: Some("演示应用".to_owned()),
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("aio.toml").is_file());
        assert!(path.join("index.html").is_file());
        assert!(path.join("rust-toolchain.toml").is_file());
        assert!(!path.join("src/pages").exists());
        assert!(path.join("src/server.rs").is_file());
        assert!(path.join("Dockerfile").is_file());
        assert!(!fs::read_to_string(path.join("src/plugins.rs"))?.contains("home::register"));
        assert!(!fs::read_to_string(path.join("src/main.rs"))?.contains("mod pages"));
        Ok(())
    }

    #[test]
    fn initializes_repository_plugin_with_client_and_server() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-plugin");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("问候".to_owned()),
            template: PluginTemplate::Rust,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("client/src/lib.rs").is_file());
        assert!(path.join("server/src/lib.rs").is_file());
        assert!(path.join("client/README.md").is_file());
        assert!(path.join("server/README.md").is_file());
        assert!(fs::read_to_string(path.join("client/src/lib.rs"))?.contains("menu_path"));
        assert!(
            fs::read_to_string(path.join("server/src/lib.rs"))?
                .contains("register(builder: &mut CatalogBuilder) -> Result<()>")
        );
        let manifest = fs::read_to_string(path.join("aio-plugin.toml"))?;
        assert!(manifest.contains("plugin.client"));
        assert!(manifest.contains("plugin.server"));
        Ok(())
    }

    #[test]
    fn initializes_kotlin_toolchain_process_plugin() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-kotlin");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("Kotlin 问候".to_owned()),
            template: PluginTemplate::KotlinService,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("kotlin").is_file());
        assert!(path.join("project.yaml").is_file());
        assert!(path.join("model/README.md").is_file());
        assert!(path.join("service/README.md").is_file());
        assert!(fs::read_to_string(path.join("aio-plugin.toml"))?.contains("kind = \"process\""));
        assert_marketplace_title(&path, "Kotlin 问候")?;
        assert!(
            fs::read_to_string(
                path.join("model/src/site/addzero/aio/plugin/hello_kotlin/RuntimeModel.kt")
            )?
            .contains("Kotlin 问候")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_ne!(
                fs::metadata(path.join("kotlin"))?.permissions().mode() & 0o111,
                0
            );
        }
        Ok(())
    }

    #[test]
    fn initializes_kotlin_toolchain_page_plugin() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-kotlin-pages");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("Kotlin 页面".to_owned()),
            template: PluginTemplate::KotlinPages,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("kotlin").is_file());
        assert!(path.join("generator/README.md").is_file());
        assert!(path.join("model/README.md").is_file());
        assert!(
            fs::read_to_string(path.join("aio-plugin.toml"))?
                .contains("kind = \"page-definition\"")
        );
        assert_marketplace_title(&path, "Kotlin 页面")?;
        let source = fs::read_to_string(
            path.join("model/src/site/addzero/aio/plugin/hello_kotlin_pages/PageDefinition.kt"),
        )?;
        assert!(source.contains("hello-kotlin-pages"));
        assert!(source.contains("Kotlin 页面"));
        Ok(())
    }

    #[test]
    fn initializes_kotlin_toolchain_component_plugin() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-kotlin-component");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("Kotlin Component 问候".to_owned()),
            template: PluginTemplate::KotlinComponent,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("kotlin").is_file());
        assert!(path.join("wit/page.wit").is_file());
        assert!(path.join("scripts/build-component.sh").is_file());
        assert!(path.join("runtime/sandbox-preview1-adapter.wat").is_file());
        assert!(
            fs::read_to_string(path.join("aio-plugin.toml"))?.contains("kind = \"wasm-component\"")
        );
        assert_marketplace_title(&path, "Kotlin Component 问候")?;
        let source = fs::read_to_string(path.join(
            "model/src/site/addzero/aio/plugin/hello_kotlin_component/contract/PluginContract.kt",
        ))?;
        assert!(source.contains("hello-kotlin-component"));
        assert!(source.contains("Kotlin Component 问候"));
        assert!(!source.contains("__PACKAGE_NAME__"));
        Ok(())
    }

    #[test]
    fn initializes_typescript_component_plugin() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-typescript");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("TypeScript 问候".to_owned()),
            template: PluginTemplate::TypeScriptComponent,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("pnpm-lock.yaml").is_file());
        assert!(path.join("wit/page.wit").is_file());
        assert!(path.join("src/plugin/README.md").is_file());
        assert!(path.join("test/plugin/README.md").is_file());
        assert!(
            fs::read_to_string(path.join("aio-plugin.toml"))?.contains("kind = \"wasm-component\"")
        );
        assert_marketplace_title(&path, "TypeScript 问候")?;
        let source = fs::read_to_string(path.join("src/plugin/component.ts"))?;
        assert!(source.contains("hello-typescript"));
        assert!(source.contains("TypeScript 问候"));
        Ok(())
    }

    #[test]
    fn initializes_typescript_process_plugin() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-node");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("Node 问候".to_owned()),
            template: PluginTemplate::TypeScriptService,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("pnpm-lock.yaml").is_file());
        assert!(path.join("src/service/README.md").is_file());
        assert!(path.join("test/service/README.md").is_file());
        assert!(fs::read_to_string(path.join("aio-plugin.toml"))?.contains("kind = \"process\""));
        assert_marketplace_title(&path, "Node 问候")?;
        let source = fs::read_to_string(path.join("src/service/server.ts"))?;
        assert!(source.contains("hello-node"));
        assert!(source.contains("Node 问候"));
        assert!(source.contains("action.kind !== \"page_action\""));
        Ok(())
    }

    #[test]
    fn initializes_typescript_page_definition_plugin() -> Result<()> {
        let root = tempdir()?;
        let path = root.path().join("hello-ts-pages");

        repository_plugin(RepositoryPluginOptions {
            path: path.clone(),
            name: None,
            title: Some("TypeScript 页面".to_owned()),
            template: PluginTemplate::TypeScriptPages,
            network: NetworkProfile::default(),
        })?;

        assert!(path.join("pnpm-lock.yaml").is_file());
        assert!(path.join("src/pages/README.md").is_file());
        assert!(path.join("src/generator/README.md").is_file());
        assert!(
            fs::read_to_string(path.join("aio-plugin.toml"))?
                .contains("kind = \"page-definition\"")
        );
        assert_marketplace_title(&path, "TypeScript 页面")?;
        let source = fs::read_to_string(path.join("src/pages/definition.ts"))?;
        assert!(source.contains("hello-ts-pages"));
        assert!(source.contains("TypeScript 页面"));
        Ok(())
    }

    fn assert_marketplace_title(path: &Path, expected: &str) -> Result<()> {
        let manifest = az_plugin_manifest::read_manifest(path)?;
        let marketplace = manifest
            .plugin
            .marketplace
            .context("多语言模板应声明市场元数据")?;
        assert_eq!(marketplace.title, expected);
        Ok(())
    }
}
