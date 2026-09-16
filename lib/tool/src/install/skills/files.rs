use anyhow::{Context as _, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn destination(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "技能文件路径无效"
    );
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "技能目标不能包含符号链接: {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

pub(super) fn collect(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(!kind.is_symlink(), "技能包不能包含符号链接");
        let path = entry.path();
        if kind.is_dir() {
            ensure!(
                path.strip_prefix(root)?.components().count() <= 8,
                "技能目录层级过深"
            );
            collect(root, &path, files)?;
            continue;
        }
        ensure!(
            kind.is_file() && entry.metadata()?.len() <= 262144,
            "技能附件必须是至多 256 KiB 的普通文件"
        );
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .context("技能文件名不是 UTF-8")?
            .to_owned();
        files.insert(relative, fs::read(&path)?);
        ensure!(
            files.len() <= 128 && files.values().map(Vec::len).sum::<usize>() <= 1048576,
            "技能包超过 128 个文件或 1 MiB"
        );
    }
    Ok(())
}

pub(super) fn check_root(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink(),
        "技能根目录不能是符号链接"
    );
    Ok(())
}

pub(super) fn remove(root: &Path, owned: &BTreeMap<String, String>) -> Result<()> {
    check_root(root)?;
    for (relative, expected) in owned {
        let path = destination(root, relative)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if digest(&bytes) != *expected {
            println!("保留用户修改的技能文件：{}", path.display());
            continue;
        }
        fs::remove_file(&path)?;
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|p| *p != root) {
            match fs::remove_dir(directory) {
                Ok(()) => parent = directory.parent(),
                Err(error)
                    if [
                        std::io::ErrorKind::DirectoryNotEmpty,
                        std::io::ErrorKind::NotFound,
                    ]
                    .contains(&error.kind()) =>
                {
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}
