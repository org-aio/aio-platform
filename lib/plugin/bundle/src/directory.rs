use std::{collections::BTreeMap, fs, io::Read, path::Path};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

use crate::{
    Bundle, BundleManifest, MAX_BUNDLE_BYTES,
    model::{MAX_FILES, MAX_MANIFEST_BYTES},
};

impl Bundle {
    pub fn from_directory(
        root: &Path,
        manifest_path: &str,
        git: String,
        commit: String,
        version: String,
    ) -> Result<Self> {
        let root = root.canonicalize().context("插件目录不存在")?;
        let manifest = String::from_utf8(read_file(&root, manifest_path, MAX_MANIFEST_BYTES)?)?;
        let files = read_artifacts(&root, &manifest, MAX_BUNDLE_BYTES)?
            .into_iter()
            .map(|(path, bytes)| (path, STANDARD.encode(bytes)))
            .collect();
        let mut bundle = Self {
            abi_version: az_plugin_contract::ABI_VERSION,
            git,
            commit,
            version,
            manifest,
            files,
            digest: String::new(),
        };
        bundle.digest = bundle.content_digest();
        bundle.verify()?;
        Ok(bundle)
    }
}

fn checked_path(root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    crate::validate_relative_path(relative)?;
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "产物不接受符号链接: {relative}"
        );
    }
    ensure!(path.canonicalize()?.starts_with(root), "产物越过仓库目录");
    Ok(path)
}

fn read_file(root: &Path, relative: &str, limit: usize) -> Result<Vec<u8>> {
    let path = checked_path(root, relative)?;
    ensure!(path.is_file(), "产物不是普通文件: {relative}");
    let file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "文件超过配额: {relative}"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "文件超过配额: {relative}");
    Ok(bytes)
}

fn collect_directory(
    root: &Path,
    relative: &str,
    paths: &mut Vec<String>,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= 16, "产物目录超过最大深度");
    for entry in fs::read_dir(checked_path(root, relative)?)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("产物文件名不是 UTF-8"))?;
        let path = format!("{relative}/{name}");
        let kind = entry.file_type()?;
        ensure!(kind.is_file() || kind.is_dir(), "产物不接受链接或特殊文件");
        if kind.is_dir() {
            collect_directory(root, &path, paths, depth + 1)?;
        } else {
            paths.push(path);
            ensure!(paths.len() <= MAX_FILES, "产物文件数量超过配额");
        }
    }
    Ok(())
}

fn read_artifacts(root: &Path, manifest: &str, limit: usize) -> Result<BTreeMap<String, Vec<u8>>> {
    let parsed = BundleManifest::parse(manifest)?;
    let mut paths = vec![parsed.plugin.runtime.artifact.clone()];
    collect_directory(root, &parsed.plugin.frontend.path, &mut paths, 0)?;
    if let Some(database) = &parsed.plugin.database {
        for entry in fs::read_dir(checked_path(root, &database.migrations)?)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sql")
            {
                ensure!(entry.file_type()?.is_file(), "迁移必须是普通 SQL 文件");
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("迁移文件名不是 UTF-8"))?;
                paths.push(format!("{}/{name}", database.migrations));
                ensure!(paths.len() <= MAX_FILES, "产物文件数量超过配额");
            }
        }
    }
    let mut remaining = limit;
    let mut files = BTreeMap::new();
    for path in paths {
        let bytes = read_file(root, &path, remaining)?;
        remaining -= bytes.len();
        ensure!(files.insert(path, bytes).is_none(), "产物路径重复");
    }
    Ok(files)
}

impl crate::VerifiedBundle {
    /// 开发控制面读取本地快照，复用正式产物校验；没有 Git 提交和市场归档。
    pub fn from_development_directory(root: &Path, digest: String) -> Result<Self> {
        ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "开发摘要无效"
        );
        let root = root.canonicalize()?;
        let manifest = String::from_utf8(read_file(&root, "aio-plugin.toml", MAX_MANIFEST_BYTES)?)?;
        let files = read_artifacts(&root, &manifest, MAX_BUNDLE_BYTES * 8)?;
        crate::validation::verify_artifacts(BundleManifest::parse(&manifest)?, files, digest, true)
    }
}
