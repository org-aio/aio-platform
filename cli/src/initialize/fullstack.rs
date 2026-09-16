use std::{fs, path::Path};

use anyhow::Result;
use include_dir::{Dir, include_dir};

use super::{PluginLanguage, WebFramework};

static RUST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/fullstack/rust");
static KOTLIN: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/fullstack/kotlin");
static TYPESCRIPT: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/fullstack/typescript");

static WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/fullstack/web");
static NUXT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/fullstack/nuxt");
static NEXT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/fullstack/next");

pub(super) fn materialize_web(
    root: &Path,
    framework: WebFramework,
    name: &str,
    title: &str,
) -> Result<()> {
    materialize_directory(root, &WEB, name, title)?;
    materialize_directory(
        root,
        match framework {
            WebFramework::Nuxt => &NUXT,
            WebFramework::Next => &NEXT,
        },
        name,
        title,
    )
}

pub(super) fn materialize(
    root: &Path,
    language: PluginLanguage,
    name: &str,
    title: &str,
) -> Result<()> {
    let template = match language {
        PluginLanguage::Rust => &RUST,
        PluginLanguage::Kotlin => &KOTLIN,
        PluginLanguage::TypeScript => &TYPESCRIPT,
    };
    materialize_directory(root, template, name, title)
}

pub(super) fn materialize_directory(
    root: &Path,
    template: &Dir<'_>,
    name: &str,
    title: &str,
) -> Result<()> {
    let identifier = name.replace('-', "_");
    let title_json = serde_json::to_string(title)?;
    let escaped_title = &title_json[1..title_json.len() - 1];
    fn visit(root: &Path, dir: &Dir<'_>, name: &str, identifier: &str, title: &str) -> Result<()> {
        for file in dir.files() {
            let relative = file
                .path()
                .to_string_lossy()
                .replace("__NAME__", name)
                .replace("/example/", &format!("/{identifier}/"));
            let destination = root.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = std::str::from_utf8(file.contents())?
                .replace("__TITLE__", title)
                .replace("__NAME__", name)
                .replace("__AIO_VERSION__", env!("CARGO_PKG_VERSION"))
                .replace("dioxus-fullstack-counter", name)
                .replace("kmp-fullstack", name)
                .replace("Dioxus Fullstack Counter", title)
                .replace("Dioxus 全栈计数器", title)
                .replace("KMP 全栈示例", title)
                .replace("fullstack-frontend", &format!("{name}-frontend"))
                .replace("fullstack-backend", &format!("{name}-backend"))
                .replace("fullstack-model", &format!("{name}-model"))
                .replace("fullstack_frontend", &format!("{identifier}_frontend"))
                .replace("fullstack_backend", &format!("{identifier}_backend"))
                .replace("fullstack_model", &format!("{identifier}_model"))
                .replace(
                    "site.addzero.aio.example",
                    &format!("site.addzero.aio.{identifier}"),
                );
            super::write(&destination, &content)?;
            #[cfg(unix)]
            if file.path().file_name().is_some_and(|name| name == "kotlin") {
                use std::os::unix::fs::PermissionsExt as _;
                fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
            }
        }
        for dir in dir.dirs() {
            visit(root, dir, name, identifier, title)?;
        }
        Ok(())
    }
    visit(root, template, name, &identifier, escaped_title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_framework_projects_with_valid_manifests_and_locked_dependencies() -> Result<()> {
        for framework in [WebFramework::Nuxt, WebFramework::Next] {
            let root = tempfile::tempdir()?;
            materialize_web(root.path(), framework, "counter-example", "计数 \"A\"")?;
            let manifest = az_plugin_manifest::read_manifest(root.path())?;
            assert_eq!(
                manifest.plugin.runtime.as_ref().unwrap().artifact,
                "dist/server.cjs"
            );
            assert_eq!(
                manifest.plugin.marketplace.as_ref().unwrap().title,
                "计数 \"A\""
            );
            assert_eq!(manifest.plugin.subplugins[0].routes, ["api/counter"]);
            let package: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(root.path().join("package.json"))?)?;
            assert_eq!(package["name"], "counter-example");
            let lock: serde_yaml::Value =
                serde_yaml::from_str(&fs::read_to_string(root.path().join("pnpm-lock.yaml"))?)?;
            for (name, version) in package["dependencies"].as_object().unwrap() {
                assert_eq!(
                    lock["importers"]["."]["dependencies"][name]["specifier"].as_str(),
                    version.as_str()
                );
            }
            fs::read_to_string(root.path().join("aio-dev.toml"))?
                .parse::<toml_edit::DocumentMut>()?;
        }
        Ok(())
    }

    #[test]
    fn creates_three_opted_in_fullstack_projects() -> Result<()> {
        for language in [
            PluginLanguage::Rust,
            PluginLanguage::Kotlin,
            PluginLanguage::TypeScript,
        ] {
            let root = std::env::temp_dir().join(format!(
                "aio-fullstack-{}-{:?}",
                std::process::id(),
                language
            ));
            if root.exists() {
                fs::remove_dir_all(&root)?;
            }
            materialize(&root, language, "delivery-example", "Delivery Example")?;
            let marker = fs::read_to_string(root.join("aio-delivery.toml"))?
                .parse::<toml_edit::DocumentMut>()?;
            assert_eq!(marker["version"].as_integer(), Some(1));
            assert!(
                marker["build"]["command"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
            );
            assert!(root.join("frontend").is_dir());
            assert!(root.join("backend").is_dir());
            assert!(root.join("shared").is_dir());
            assert!(!root.join(".github/workflows").exists());
            let manifest = az_plugin_manifest::read_manifest(&root)?;
            assert!(manifest.plugin.runtime.is_some());
            assert!(manifest.plugin.frontend.is_some());
            if language == PluginLanguage::Kotlin {
                assert!(
                    az_plugin_manifest::validate_host_compatibility(&manifest, "2026.9.11")
                        .is_err()
                );
                az_plugin_manifest::validate_host_compatibility(&manifest, "2026.9.14")?;
                let pages = az_plugin_manifest::parse_page_definitions(&fs::read(
                    root.join("backend/service/resources/pages.json"),
                )?)?;
                assert_eq!(pages.len(), 1);
                assert_eq!(pages[0].id, "delivery-example");
                assert_eq!(pages[0].label, "Delivery Example");
            }
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }
}
