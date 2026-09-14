use std::collections::BTreeMap;

use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{MAX_FRONTEND_FILES, RepositoryManifest, validate_frontend_path};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest as _, Sha256};

use crate::{MAX_BUNDLE_BYTES, PluginPackage};

impl PluginPackage {
    pub(super) fn verify_frontend(
        &self,
        manifest: &RepositoryManifest,
        artifact_bytes: usize,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        ensure!(
            self.frontend.len() <= MAX_FRONTEND_FILES,
            "前端资产数量超过 {MAX_FRONTEND_FILES}"
        );
        let Some(frontend) = &manifest.plugin.frontend else {
            ensure!(self.frontend.is_empty(), "插件包包含未声明的前端资产");
            return Ok(BTreeMap::new());
        };
        ensure!(
            !self.frontend.is_empty(),
            "声明了前端能力但包内没有前端资产"
        );
        let runtime = manifest
            .plugin
            .runtime
            .as_ref()
            .context("前端资产缺少运行产物")?;
        let mut total = artifact_bytes;
        let mut verified = BTreeMap::new();
        let paths = self
            .frontend
            .keys()
            .map(|path| path.to_ascii_lowercase())
            .collect::<std::collections::BTreeSet<_>>();
        ensure!(
            paths.len() == self.frontend.len(),
            "前端资产存在大小写路径冲突"
        );
        for (path, asset) in &self.frontend {
            validate_frontend_path(path)?;
            // 两个文件不能同时占据目录与普通文件的位置。
            let lower_path = path.to_ascii_lowercase();
            let mut prefix = lower_path.as_str();
            while let Some((parent, _)) = prefix.rsplit_once('/') {
                ensure!(!paths.contains(parent), "前端文件路径冲突: {path}");
                prefix = parent;
            }
            let destination = format!("{}/{path}", frontend.path).to_ascii_lowercase();
            for reserved in [&runtime.artifact, "aio-plugin.toml"] {
                let reserved = reserved.to_ascii_lowercase();
                ensure!(
                    destination != reserved
                        && !destination.starts_with(&format!("{reserved}/"))
                        && !reserved.starts_with(&format!("{destination}/")),
                    "前端资产与清单或后端产物冲突: {path}"
                );
            }
            ensure!(
                asset.content_base64.len()
                    <= MAX_BUNDLE_BYTES.saturating_sub(total).div_ceil(3) * 4,
                "前后端产物合计超过 64 MiB"
            );
            let bytes = STANDARD
                .decode(&asset.content_base64)
                .context("前端资产 base64 无效")?;
            total = total.checked_add(bytes.len()).context("前端资产大小溢出")?;
            ensure!(total <= MAX_BUNDLE_BYTES, "前后端产物合计超过 64 MiB");
            ensure!(
                asset.sha256 == format!("{:x}", Sha256::digest(&bytes)),
                "前端资产 SHA-256 校验失败: {path}"
            );
            verified.insert(path.clone(), bytes);
        }
        if runtime.kind == az_plugin_manifest::PluginRuntime::PageDefinition {
            let artifact = STANDARD
                .decode(&self.artifact_base64)
                .context("页面产物 base64 无效")?;
            let pages = az_plugin_manifest::parse_page_definitions(&artifact)
                .context("解析前端页面定义失败")?;
            az_plugin_manifest::validate_frontend_pages(
                manifest,
                &pages,
                verified.keys().map(String::as_str),
            )?;
        }
        Ok(verified)
    }
}

#[cfg(test)]
#[path = "frontend_tests.rs"]
mod tests;
