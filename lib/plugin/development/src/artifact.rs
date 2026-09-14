use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::Path;

/// 以角色和相对路径计算产物身份，不受工作区路径或发布目录布局影响。
pub fn artifact_digest(manifest: &Path, frontend: &Path, backend: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    for (role, path) in [
        ("manifest", manifest),
        ("frontend", frontend),
        ("backend", backend),
    ] {
        digest.update(role.as_bytes());
        visit(path, path, &mut digest)?;
    }
    let configuration: toml::Value = toml::from_str(&std::fs::read_to_string(manifest)?)?;
    if let Some(directory) = configuration
        .get("plugin")
        .and_then(|p| p.get("database"))
        .and_then(|d| d.get("migrations"))
        .and_then(|m| m.as_str())
    {
        crate::validate_relative(directory)?;
        digest.update(b"migrations");
        let path = manifest
            .parent()
            .ok_or_else(|| anyhow::anyhow!("清单路径无效"))?
            .join(directory);
        visit(&path, &path, &mut digest)?;
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn backend_digest(path: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    visit(path, path, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

fn visit(root: &Path, path: &Path, digest: &mut Sha256) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(!metadata.file_type().is_symlink(), "产物不能包含符号链接");
    digest.update(path.strip_prefix(root)?.to_string_lossy().as_bytes());
    digest.update([0]);
    if metadata.is_dir() {
        digest.update(b"directory\0");
        let mut entries = std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        for entry in entries {
            visit(root, &entry, digest)?;
        }
    } else {
        ensure!(metadata.is_file(), "产物只能包含普通文件和目录");
        digest.update(b"file\0");
        digest.update(metadata.len().to_le_bytes());
        digest.update(std::fs::read(path)?);
    }
    digest.update([0]);
    Ok(())
}
