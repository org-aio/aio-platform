use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use crate::{Bundle, BundleManifest, MAX_BUNDLE_BYTES, VerifiedBundle, model::MAX_FILES};

pub fn validate_relative_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 1024
            && path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@/".contains(&byte))
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "产物路径必须是安全的包内相对路径: {path}"
    );
    Ok(())
}

impl Bundle {
    pub fn verify(&self) -> Result<VerifiedBundle> {
        ensure!(
            self.abi_version == az_plugin_contract::ABI_VERSION,
            "插件 ABI 不是 v2"
        );
        let git = url::Url::parse(&self.git)?;
        ensure!(
            git.scheme() == "https"
                && git.host_str().is_some()
                && git.username().is_empty()
                && git.password().is_none()
                && git.query().is_none()
                && git.fragment().is_none(),
            "Git 来源必须是无凭据的 HTTPS 地址"
        );
        ensure!(
            self.commit.len() == 40
                && self
                    .commit
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "Git 来源必须锁定完整的 40 位小写提交 SHA"
        );
        semver::Version::parse(&self.version)?;
        ensure!(self.files.len() <= MAX_FILES, "产物文件数量超过配额");
        let manifest = BundleManifest::parse(&self.manifest)?;
        let plugin = &manifest.plugin;
        ensure!(
            plugin
                .marketplace
                .as_ref()
                .and_then(|m| m.parent.as_deref())
                != Some(self.git.as_str()),
            "插件不能以自身为父插件"
        );
        ensure!(self.digest == self.content_digest(), "整包摘要不匹配");
        let mut remaining = MAX_BUNDLE_BYTES;
        let mut files = BTreeMap::new();
        for (path, encoded) in &self.files {
            // 解码前限额，避免临时分配绕过整包配额。
            ensure!(
                encoded.len() <= remaining.div_ceil(3) * 4,
                "产物超过整包配额"
            );
            let bytes = STANDARD.decode(encoded).context("产物 Base64 无效")?;
            ensure!(bytes.len() <= remaining, "产物超过整包配额");
            remaining -= bytes.len();
            files.insert(path.clone(), bytes);
        }
        verify_artifacts(manifest, files, self.digest.clone(), false)
    }

    pub(crate) fn content_digest(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"aio:plugin@2.0.0/bundle\0");
        hash.update(self.abi_version.to_be_bytes());
        for value in [&self.git, &self.commit, &self.version, &self.manifest] {
            hash_field(&mut hash, value.as_bytes());
        }
        for (path, content) in &self.files {
            hash_field(&mut hash, path.as_bytes());
            hash_field(&mut hash, content.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
}

fn hash_field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

pub(crate) fn verify_artifacts(
    manifest: BundleManifest,
    files: BTreeMap<String, Vec<u8>>,
    digest: String,
    development: bool,
) -> Result<VerifiedBundle> {
    ensure!(files.len() <= MAX_FILES, "产物文件数量超过配额");
    let plugin = &manifest.plugin;
    let frontend_prefix = format!("{}/", plugin.frontend.path);
    let migration_prefix = plugin
        .database
        .as_ref()
        .map(|value| format!("{}/", value.migrations));
    let mut remaining = if development {
        MAX_BUNDLE_BYTES * 8
    } else {
        MAX_BUNDLE_BYTES
    };
    let mut frontend_assets = BTreeMap::new();
    let mut frontend_count = 0;
    let mut migration_count = 0;
    for (path, bytes) in &files {
        validate_relative_path(path)?;
        for (offset, _) in path.match_indices('/') {
            ensure!(
                !files.contains_key(&path[..offset]),
                "文件和目录路径冲突: {path}"
            );
        }
        ensure!(bytes.len() <= remaining, "产物超过整包配额");
        remaining -= bytes.len();
        if path == &plugin.runtime.artifact {
            if plugin.runtime.process.is_some() && !development {
                ensure!(
                    bytes.len() >= 64
                        && bytes.starts_with(b"\x7fELF\x02\x01")
                        && bytes[18..20] == [62, 0],
                    "process 后端必须是 Linux x86_64 ELF"
                );
            } else if plugin.runtime.process.is_none() {
                ensure!(
                    bytes.starts_with(b"\0asm\x0d\0\x01\0"),
                    "后端不是 Wasm Component，不能装入浏览器 Wasm 或 JAR"
                );
            }
        } else if path.starts_with(&frontend_prefix) {
            frontend_count += 1;
            frontend_assets.insert(
                path[frontend_prefix.len()..].to_owned(),
                crate::FrontendAsset {
                    digest: format!("{:x}", Sha256::digest(&bytes)),
                    size: bytes.len(),
                },
            );
        } else if let Some(relative) = migration_prefix
            .as_deref()
            .and_then(|prefix| path.strip_prefix(prefix))
        {
            ensure!(
                !relative.contains('/') && relative.ends_with(".sql"),
                "迁移目录只接受直接包含的 SQL 文件"
            );
            ensure!(bytes.len() <= 1024 * 1024, "单个迁移文件超过配额");
            ensure!(
                !std::str::from_utf8(&bytes)?.trim().is_empty(),
                "迁移文件不能为空"
            );
            migration_count += 1;
        } else {
            anyhow::bail!("包包含未声明产物: {path}");
        }
    }
    ensure!(
        files.contains_key(&plugin.runtime.artifact),
        "包缺少后端产物"
    );
    ensure!(frontend_count > 0, "全栈包缺少前端产物");
    ensure!(
        plugin.database.is_none() || migration_count > 0,
        "包缺少已声明的数据库迁移"
    );
    Ok(VerifiedBundle {
        manifest,
        files,
        digest,
        frontend_assets,
    })
}
