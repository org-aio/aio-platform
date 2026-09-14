use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::path::Path;

/// 内容指纹包含相对路径、缺失输入和文件内容，不使用 mtime 代替源码身份。
pub fn fingerprint(root: &Path, inputs: &[String]) -> Result<String> {
    fn visit(root: &Path, path: &Path, digest: &mut Sha256) -> Result<()> {
        digest.update(path.strip_prefix(root)?.to_string_lossy().as_bytes());
        digest.update([0]);
        if !path.exists() {
            digest.update(b"missing\0");
            return Ok(());
        }
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            !meta.file_type().is_symlink(),
            "开发输入不能是符号链接: {}",
            path.display()
        );
        if meta.is_dir() {
            let mut children = std::fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            children.sort();
            for child in children {
                if child.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                    matches!(
                        s,
                        ".git"
                            | ".aio"
                            | "node_modules"
                            | "target"
                            | "build"
                            | "dist"
                            | ".DS_Store"
                            | "aio-dev.lock"
                    )
                }) {
                    continue;
                }
                visit(root, &child, digest)?;
            }
        } else {
            digest.update(meta.len().to_le_bytes());
            digest.update(
                std::fs::read(path).with_context(|| format!("读取输入失败: {}", path.display()))?,
            );
        }
        digest.update([0]);
        Ok(())
    }
    let mut digest = Sha256::new();
    let mut inputs = inputs.to_vec();
    inputs.sort();
    inputs.dedup();
    for input in inputs {
        crate::validate_relative(&input)?;
        visit(root, &root.join(input), &mut digest)?;
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn source_identity(root: &Path) -> Result<String> {
    let root = root.canonicalize()?;
    Ok(url::Url::from_directory_path(root)
        .map_err(|_| anyhow::anyhow!("工作区路径无效"))?
        .into())
}
