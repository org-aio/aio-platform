use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, ensure};
use az_plugin_bundle::{Bundle, MAX_ENCODED_BYTES};
use az_plugin_package::{
    MAX_ARTIFACT_BYTES, MAX_BUNDLE_BYTES, MAX_MANIFEST_BYTES, MAX_PACKAGE_BYTES, PluginPackage,
    normalize_git_source,
};

#[derive(Debug)]
pub struct PackageOptions {
    pub root: PathBuf,
    pub git: Option<String>,
    pub version: String,
    pub output: Option<PathBuf>,
}

pub fn package(options: PackageOptions) -> Result<()> {
    let package = prepare_release(&options.root, options.git, Some(options.version))?;
    let output = options
        .output
        .unwrap_or_else(|| options.root.join("dist/plugin.aio-plugin"));
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建插件包输出目录失败: {}", parent.display()))?;
    }
    let bytes = package.encode()?;
    fs::write(&output, &bytes).with_context(|| format!("写入插件包失败: {}", output.display()))?;
    println!(
        "插件已打包: {} version={} revision={} bytes={}",
        output.display(),
        package.version(),
        package.revision(),
        bytes.len()
    );
    Ok(())
}

#[derive(Debug)]
pub(super) enum PreparedRelease {
    Legacy(PluginPackage),
    Bundle(Bundle),
}

impl PreparedRelease {
    pub(super) fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Legacy(package) => package.encode(),
            Self::Bundle(bundle) => bundle.encode(),
        }
    }

    pub(super) fn git(&self) -> &str {
        match self {
            Self::Legacy(package) => &package.git,
            Self::Bundle(bundle) => &bundle.git,
        }
    }

    pub(super) fn version(&self) -> &str {
        match self {
            Self::Legacy(package) => &package.version,
            Self::Bundle(bundle) => &bundle.version,
        }
    }

    pub(super) fn revision(&self) -> &str {
        match self {
            Self::Legacy(package) => &package.rev,
            Self::Bundle(bundle) => &bundle.digest,
        }
    }
}

pub(super) fn uses_bundle_manifest(root: &Path) -> Result<bool> {
    let manifest = String::from_utf8(read_bounded(
        &root.join("aio-plugin.toml"),
        MAX_MANIFEST_BYTES,
    )?)
    .context("aio-plugin.toml 必须使用 UTF-8 编码")?;
    let manifest: toml::Value = toml::from_str(&manifest).context("解析插件清单失败")?;
    Ok(manifest
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        == Some(2))
}

pub(super) fn prepare_release(
    root: &Path,
    git: Option<String>,
    version: Option<String>,
) -> Result<PreparedRelease> {
    if !uses_bundle_manifest(root)? {
        return Ok(PreparedRelease::Legacy(prepare_package(
            root, git, version,
        )?));
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("解析插件目录失败: {}", root.display()))?;
    let repository = own_git_repository(&root);
    let git = git
        .or_else(|| {
            repository
                .then(|| git_text(&root, &["remote", "get-url", "origin"]))
                .flatten()
        })
        .context("没有可用的 HTTPS Git 来源，请指定 --git <URL>")?;
    let git = normalize_git_source(&git)?;
    let version = version
        .or_else(|| project_version(&root))
        .context("缺少插件发布版本，请指定 --version <SemVer>")?;
    // 独立目录没有提交，使用整包协议允许的零 SHA 保留无 Git 打包行为。
    let revision = repository
        .then(|| git_text(&root, &["rev-parse", "HEAD"]))
        .flatten()
        .unwrap_or_else(|| "0".repeat(40));
    let bundle = Bundle::from_directory(&root, "aio-plugin.toml", git, revision, version)?;
    Ok(PreparedRelease::Bundle(bundle))
}

pub(super) fn prepare_package(
    root: &Path,
    git: Option<String>,
    version: Option<String>,
) -> Result<PluginPackage> {
    let root = root
        .canonicalize()
        .with_context(|| format!("解析插件目录失败: {}", root.display()))?;
    ensure!(root.is_dir(), "插件打包输入必须是目录");
    let manifest_toml = String::from_utf8(read_bounded(
        &root.join("aio-plugin.toml"),
        MAX_MANIFEST_BYTES,
    )?)
    .context("aio-plugin.toml 必须使用 UTF-8 编码")?;
    let manifest = az_plugin_manifest::parse_manifest(&manifest_toml)?;
    let runtime = manifest
        .plugin
        .runtime
        .as_ref()
        .context("Rust 源码插件不能直接在线发布，请先构建可独立运行的产物")?;
    let artifact_path = az_plugin_manifest::artifact_path(&root, &runtime.artifact)?;
    let artifact = read_bounded(&artifact_path, MAX_ARTIFACT_BYTES)?;
    let mut frontend = std::collections::BTreeMap::new();
    let mut remaining = MAX_BUNDLE_BYTES - artifact.len();
    for (relative, path) in az_plugin_manifest::frontend_files(&root, &manifest)? {
        let content = read_bounded(&path, remaining)?;
        remaining -= content.len();
        frontend.insert(relative, content);
    }
    az_plugin_manifest::validate_repository(&root)?;
    let repository = own_git_repository(&root);
    let git = git
        .or_else(|| {
            repository
                .then(|| git_text(&root, &["remote", "get-url", "origin"]))
                .flatten()
        })
        .context("没有可用的 HTTPS Git 来源，请指定 --git <URL>")?;
    let version = version
        .or_else(|| project_version(&root))
        .context("缺少插件发布版本，请指定 --version <SemVer>")?;
    let source_revision = repository
        .then(|| git_text(&root, &["rev-parse", "HEAD"]))
        .flatten();
    PluginPackage::new(
        git,
        version,
        source_revision,
        manifest_toml,
        &artifact,
        frontend,
    )
}

pub(super) fn read_release(
    path: &Path,
    git: Option<&str>,
    version: Option<&str>,
) -> Result<PreparedRelease> {
    let bytes = read_bounded(path, MAX_PACKAGE_BYTES.max(MAX_ENCODED_BYTES))?;
    let package = match Bundle::decode(&bytes) {
        Ok(bundle) => PreparedRelease::Bundle(bundle),
        Err(bundle_error) => {
            let legacy = PluginPackage::decode(&bytes).with_context(|| {
                format!(
                    "插件包既不是 AIO v2 整包，也不是旧版二进制包: {}；整包校验失败: {bundle_error:#}",
                    path.display()
                )
            })?;
            PreparedRelease::Legacy(legacy)
        }
    };
    if let Some(git) = git {
        ensure!(
            package.git() == normalize_git_source(git)?,
            "--git 与插件包中的来源不一致，请重新打包"
        );
    }
    if let Some(version) = version {
        ensure!(
            package.version() == version,
            "--version 与插件包中的版本不一致，请重新打包"
        );
    }
    Ok(package)
}

fn project_version(root: &Path) -> Option<String> {
    if let Ok(content) = fs::read_to_string(root.join("Cargo.toml"))
        && let Ok(cargo) = toml::from_str::<toml::Value>(&content)
    {
        if let Some(version) = cargo
            .get("package")
            .and_then(|package| package.get("version"))
            .and_then(toml::Value::as_str)
        {
            return Some(version.to_owned());
        }
        if let Some(version) = cargo
            .get("workspace")
            .and_then(|workspace| workspace.get("package"))
            .and_then(|package| package.get("version"))
            .and_then(toml::Value::as_str)
        {
            return Some(version.to_owned());
        }
    }
    let content = fs::read_to_string(root.join("package.json")).ok()?;
    let package: serde_json::Value = serde_json::from_str(&content).ok()?;
    package.get("version")?.as_str().map(str::to_owned)
}

fn own_git_repository(root: &Path) -> bool {
    git_text(root, &["rev-parse", "--show-toplevel"])
        .and_then(|path| PathBuf::from(path).canonicalize().ok())
        .is_some_and(|repository| repository == root)
}

fn git_text(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .map(|value| value.trim().to_owned())
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file =
        fs::File::open(path).with_context(|| format!("读取插件文件失败: {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "插件文件超过 {} 字节: {}",
        limit,
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("读取插件文件失败: {}", path.display()))?;
    ensure!(
        bytes.len() <= limit,
        "插件文件超过 {} 字节: {}",
        limit,
        path.display()
    );
    Ok(bytes)
}

#[cfg(test)]
#[path = "packaging_tests.rs"]
mod tests;
