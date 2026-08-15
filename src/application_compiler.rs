use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    ApplicationBundle, ApplicationGenerationResult, ApplicationSourceFile, PageRendererDefinition,
    ProgramDefinition, ProgramImage, convention_page_path,
    convention_source::{convention_page_source, convention_pages_source},
};

const GENERATED_MARKER: &str = ".aio-generated";

/// 把不可变 Revision 编译为独立、可复现的跨平台应用源码。
#[derive(Clone, Debug, Default)]
pub struct ApplicationCompiler;

impl ApplicationCompiler {
    pub fn compile(
        &self,
        definition: &ProgramDefinition,
        image: &ProgramImage,
    ) -> Result<ApplicationBundle> {
        validate_application_id(&definition.name)?;
        ensure!(
            !definition.application_targets.is_empty(),
            "应用至少需要一个客户端发布目标"
        );
        let package_name = format!("az-app-{}", definition.name);
        let targets = definition
            .application_targets
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let mut files = vec![
            source_file(GENERATED_MARKER, "AIO generated application\n"),
            source_file(
                "Cargo.toml",
                cargo_toml(&definition.name, &package_name, &definition.title),
            ),
            source_file("Dioxus.toml", dioxus_toml(&package_name, &definition.title)),
            source_file("src/main.rs", main_source()),
            source_file("src/pages.rs", convention_pages_source(definition)),
            source_file(
                "program-definition.json",
                pretty_json(definition).context("序列化 ProgramDefinition 失败")?,
            ),
            source_file(
                "program-image.json",
                pretty_json(image).context("序列化 ProgramImage 失败")?,
            ),
            source_file(".env.example", env_example()),
            source_file("Dockerfile", dockerfile(&package_name)),
            source_file(
                "README.md",
                readme(&definition.name, &definition.title, &package_name),
            ),
        ];
        for page in &definition.pages {
            if matches!(page.renderer, PageRendererDefinition::ConventionFile) {
                files.push(source_file(
                    convention_page_path(&definition.name, &page.name),
                    convention_page_source(page),
                ));
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(ApplicationBundle {
            application_id: definition.name.clone(),
            title: definition.title.clone(),
            revision_id: image.revision_id.clone(),
            content_hash: image.content_hash.clone(),
            targets,
            files,
        })
    }
}

/// 只管理仓库 generated/apps 目录中带生成标记的应用，拒绝覆盖手写项目。
#[derive(Clone, Debug)]
pub struct ApplicationWorkspace {
    root: PathBuf,
}

impl ApplicationWorkspace {
    #[must_use]
    pub fn repository() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir
            .ancestors()
            .nth(3)
            .map(Path::to_path_buf)
            .unwrap_or(manifest_dir);
        Self { root }
    }

    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn write(&self, bundle: &ApplicationBundle) -> Result<ApplicationGenerationResult> {
        validate_application_id(&bundle.application_id)?;
        let generated_apps_dir = self.root.join("generated/apps");
        fs::create_dir_all(&generated_apps_dir).context("创建生成应用目录失败")?;
        let application_dir = generated_apps_dir.join(&bundle.application_id);
        ensure_direct_child(&generated_apps_dir, &application_dir)?;
        if application_dir.exists() && !application_dir.join(GENERATED_MARKER).is_file() {
            bail!(
                "拒绝覆盖未带 {} 标记的目录: {}",
                GENERATED_MARKER,
                application_dir.display()
            );
        }

        let temporary_dir =
            generated_apps_dir.join(format!(".{}.tmp-{}", bundle.application_id, Uuid::new_v4()));
        fs::create_dir(&temporary_dir).context("创建应用生成临时目录失败")?;
        let write_result = write_bundle_files(&temporary_dir, &bundle.files);
        if let Err(error) = write_result {
            let _ = fs::remove_dir_all(&temporary_dir);
            return Err(error);
        }
        if let Err(error) = format_bundle_rust_sources(&temporary_dir, &bundle.files) {
            let _ = fs::remove_dir_all(&temporary_dir);
            return Err(error);
        }

        let backup_dir = generated_apps_dir.join(format!(
            ".{}.backup-{}",
            bundle.application_id,
            Uuid::new_v4()
        ));
        if application_dir.exists() {
            fs::rename(&application_dir, &backup_dir).context("备份旧生成应用失败")?;
        }
        if let Err(error) = fs::rename(&temporary_dir, &application_dir) {
            if backup_dir.exists() {
                let _ = fs::rename(&backup_dir, &application_dir);
            }
            return Err(error).context("原子替换生成应用失败");
        }
        if backup_dir.exists() {
            fs::remove_dir_all(&backup_dir).context("删除旧生成应用备份失败")?;
        }

        Ok(ApplicationGenerationResult {
            application_id: bundle.application_id.clone(),
            path: application_dir.display().to_string(),
            revision_id: bundle.revision_id.clone(),
            content_hash: bundle.content_hash.clone(),
            targets: bundle.targets.clone(),
            files: bundle.files.iter().map(|file| file.path.clone()).collect(),
        })
    }
}

fn format_bundle_rust_sources(root: &Path, files: &[ApplicationSourceFile]) -> Result<()> {
    let paths = files
        .iter()
        .filter(|file| file.path.ends_with(".rs"))
        .map(|file| root.join(&file.path))
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
    ensure!(status.success(), "rustfmt 格式化生成应用失败");
    Ok(())
}

fn write_bundle_files(root: &Path, files: &[ApplicationSourceFile]) -> Result<()> {
    for file in files {
        let relative_path = Path::new(&file.path);
        ensure_relative_path(relative_path)?;
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建生成目录失败: {}", parent.display()))?;
        }
        fs::write(&path, &file.content)
            .with_context(|| format!("写入生成文件失败: {}", path.display()))?;
    }
    Ok(())
}

fn source_file(path: impl Into<String>, content: impl Into<String>) -> ApplicationSourceFile {
    ApplicationSourceFile {
        path: path.into(),
        content: content.into(),
    }
}

fn pretty_json(value: &impl Serialize) -> Result<String> {
    let mut output = serde_json::to_string_pretty(value)?;
    output.push('\n');
    Ok(output)
}

fn validate_application_id(value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "应用目录标识不能为空");
    ensure!(
        value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase()),
        "应用目录标识必须以小写字母开头"
    );
    ensure!(
        value.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'),
        "应用目录标识只能包含小写字母、数字和连字符"
    );
    Ok(())
}

fn ensure_relative_path(path: &Path) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "生成文件路径不能为空");
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "生成文件路径必须是应用目录内的普通相对路径: {}",
        path.display()
    );
    Ok(())
}

fn ensure_direct_child(parent: &Path, child: &Path) -> Result<()> {
    ensure!(
        child.parent() == Some(parent),
        "生成应用路径越出 generated/apps 目录"
    );
    Ok(())
}

fn cargo_toml(application_id: &str, package_name: &str, title: &str) -> String {
    let description = toml_string(&format!("{title} generated application"));
    format!(
        r#"[package]
name = "{package_name}"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
description = {description}
workspace = "../../.."

[features]
default = []
client = [
    "dep:az-ui-components",
    "dep:dioxus",
    "dep:studio",
]
web = ["client", "dioxus/web", "dioxus/launch", "studio/web"]
desktop = ["client", "dioxus/desktop", "dioxus/launch", "studio/desktop"]
server = ["dep:anyhow", "dep:az-aio-app", "dep:business"]

[dependencies]
anyhow = {{ workspace = true, optional = true }}
az-ui-components = {{ workspace = true, optional = true }}
dioxus = {{ workspace = true, optional = true }}
studio = {{ package = "az-studio", path = "../../../app/plugins/studio", default-features = false, optional = true }}

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
az-aio-app = {{ path = "../../../app", optional = true }}
business = {{ package = "az-biz-{application_id}", path = "../../../lib/biz/{application_id}", optional = true }}

[lints]
workspace = true
"#
    )
}

fn dioxus_toml(package_name: &str, title: &str) -> String {
    let title = toml_string(title);
    format!(
        r#"[application]
name = "{package_name}"
default_platform = "web"
out_dir = "dist"

[web.app]
title = {title}
base_path = "/app"
"#
    )
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("字符串序列化不会失败")
}

fn main_source() -> &'static str {
    r#"#![forbid(unsafe_code)]

#[cfg(any(feature = "web", feature = "desktop"))]
use dioxus::prelude::*;

#[cfg(any(feature = "web", feature = "desktop"))]
mod pages;
#[cfg(any(
    all(feature = "web", feature = "desktop"),
    all(feature = "web", feature = "server"),
    all(feature = "desktop", feature = "server")
))]
compile_error!("web、desktop、server 每次只能启用一个目标");

#[cfg(not(any(feature = "web", feature = "desktop", feature = "server")))]
fn main() {
    eprintln!("请选择 --features web、desktop 或 server");
}

#[cfg(any(feature = "web", feature = "desktop"))]
fn main() {
    dioxus::launch(App);
}

#[cfg(any(feature = "web", feature = "desktop"))]
#[allow(non_snake_case)]
fn App() -> Element {
    rsx! {
        studio::PublishedApplication {
            render_convention: studio::ConventionPageRenderer::new(pages::render),
            admin_enabled: false,
            user: studio::ApplicationUser {
                label: "zjarlin".to_owned(),
                handle: "@zjarlin".to_owned(),
                initials: "ZJ".to_owned(),
            },
        }
    }
}

#[cfg(feature = "server")]
fn main() -> anyhow::Result<()> {
    az_aio_app::run_server_with(business::register)
}
"#
}

fn env_example() -> &'static str {
    r#"AZ_AIO_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/aio
AZ_AIO_PORT=8080
AIO_API_BASE_URL=http://127.0.0.1:8080
"#
}

fn dockerfile(package_name: &str) -> String {
    format!(
        r#"FROM rustlang/rust:nightly-bookworm AS builder
WORKDIR /workspace
RUN cargo install dioxus-cli --version 0.7.9 --locked
COPY . .
RUN dx build --package {package_name} --platform web --release --features web --debug-symbols false
RUN cargo build -p {package_name} --release --no-default-features --features server

FROM debian:bookworm-slim
WORKDIR /opt/aio
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /workspace/target/release/{package_name} /usr/local/bin/{package_name}
COPY --from=builder /workspace/target/dx/{package_name}/release/web/public /opt/aio/web
ENV AZ_AIO_WEB_DIST=/opt/aio/web
EXPOSE 8080
CMD ["/usr/local/bin/{package_name}"]
"#
    )
}

fn readme(application_id: &str, title: &str, package_name: &str) -> String {
    format!(
        r#"# {title}

该目录由 AIO ApplicationCompiler 从已发布的 ProgramDefinition 生成。

应用标识：`{application_id}`  
Cargo 包：`{package_name}`

## Web

```bash
dx serve --package {package_name} --platform web --features web
```

## Desktop

```bash
AIO_API_BASE_URL=http://127.0.0.1:8080 cargo run -p {package_name} --no-default-features --features desktop
```

## Server

```bash
cargo run -p {package_name} --no-default-features --features server
```

## Container

从仓库根目录执行：

```bash
docker build -f generated/apps/{application_id}/Dockerfile -t {package_name} .
```
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApplicationTarget, CapabilityCatalog, ImageTarget, ProgramCompiler};

    #[test]
    fn compiler_emits_stable_cross_platform_project() -> Result<()> {
        let definition = ProgramDefinition::empty("example-app", "示例应用");
        let image = ProgramCompiler::new("test", &CapabilityCatalog::default())
            .compile(&definition, "revision-1", ImageTarget::Universal)
            .map_err(anyhow::Error::from)?;
        let first = ApplicationCompiler.compile(&definition, &image)?;
        let second = ApplicationCompiler.compile(&definition, &image)?;
        assert_eq!(first, second);
        assert_eq!(
            first.targets,
            vec![ApplicationTarget::Web, ApplicationTarget::Desktop]
        );
        let cargo = first
            .files
            .iter()
            .find(|file| file.path == "Cargo.toml")
            .context("缺少 Cargo.toml")?;
        assert!(cargo.content.contains("studio/web"));
        assert!(cargo.content.contains("studio/desktop"));
        assert!(
            cargo
                .content
                .contains("server = [\"dep:anyhow\", \"dep:az-aio-app\", \"dep:business\"]")
        );
        let main = first
            .files
            .iter()
            .find(|file| file.path == "src/main.rs")
            .context("缺少 src/main.rs")?;
        assert!(main.content.contains("admin_enabled: false"));
        assert!(main.content.contains("user: studio::ApplicationUser"));
        assert!(main.content.contains("handle: \"@zjarlin\""));
        assert!(!main.content.contains("studio_enabled"));
        Ok(())
    }

    #[test]
    fn workspace_refuses_non_generated_application_directory() -> Result<()> {
        let root = std::env::temp_dir().join(format!("aio-app-workspace-{}", Uuid::new_v4()));
        let application_dir = root.join("generated/apps/example-app");
        fs::create_dir_all(&application_dir)?;
        fs::write(application_dir.join("README.md"), "hand written")?;
        let bundle = ApplicationBundle {
            application_id: "example-app".to_owned(),
            title: "示例应用".to_owned(),
            revision_id: "revision-1".to_owned(),
            content_hash: "hash".to_owned(),
            targets: vec![ApplicationTarget::Web],
            files: vec![source_file(GENERATED_MARKER, "generated")],
        };
        let result = ApplicationWorkspace::new(root.clone()).write(&bundle);
        let _ = fs::remove_dir_all(root);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn workspace_replaces_bundle_and_removes_stale_metadata_files() -> Result<()> {
        let root = std::env::temp_dir().join(format!("aio-app-reconcile-{}", Uuid::new_v4()));
        let workspace = ApplicationWorkspace::new(root.clone());
        let first = ApplicationBundle {
            application_id: "example-app".to_owned(),
            title: "示例应用".to_owned(),
            revision_id: "revision-1".to_owned(),
            content_hash: "hash-1".to_owned(),
            targets: vec![ApplicationTarget::Web],
            files: vec![
                source_file(GENERATED_MARKER, "generated"),
                source_file("src/pages/removed.rs", "pub fn removed() {}\n"),
            ],
        };
        workspace.write(&first)?;

        let application_dir = root.join("generated/apps/example-app");
        assert!(application_dir.join("src/pages/removed.rs").is_file());

        let second = ApplicationBundle {
            revision_id: "revision-2".to_owned(),
            content_hash: "hash-2".to_owned(),
            files: vec![
                source_file(GENERATED_MARKER, "generated"),
                source_file("src/pages/current.rs", "pub fn current() {}\n"),
            ],
            ..first
        };
        workspace.write(&second)?;

        assert!(!application_dir.join("src/pages/removed.rs").exists());
        assert!(application_dir.join("src/pages/current.rs").is_file());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
