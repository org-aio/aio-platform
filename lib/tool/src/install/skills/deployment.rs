use super::super::{Store, storage::Installation};
use super::files;
use anyhow::{Context as _, Result, ensure};
use std::{collections::BTreeMap, fs, io::Write, path::Path};

pub(super) fn apply(
    store: &Store,
    files: BTreeMap<String, Vec<u8>>,
    record: &mut Installation,
) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    files::check_root(&store.skills_root)?;
    for (relative, bytes) in &files {
        let name = Path::new(relative)
            .components()
            .next()
            .context("技能路径无效")?;
        let directory = store.skills_root.join(name);
        if directory.exists() {
            ensure!(
                record
                    .skills
                    .keys()
                    .any(|owned| Path::new(owned).components().next() == Some(name)),
                "技能目录已存在且不属于此工具: {}",
                directory.display()
            );
        }
        let target = files::destination(&store.skills_root, relative)?;
        if target.exists() {
            let actual = files::digest(&fs::read(&target)?);
            ensure!(
                record.skills.get(relative) == Some(&actual),
                "技能文件已存在或被修改，拒绝覆盖: {}",
                target.display()
            );
            ensure!(
                actual == files::digest(bytes),
                "技能版本内容变化，需要先卸载旧版本"
            );
        }
    }
    // 先记录归属再写文件，进程中断后仍能按内容哈希恢复与卸载。
    for (relative, bytes) in &files {
        record.skills.insert(relative.clone(), files::digest(bytes));
    }
    store.save(record)?;
    for (relative, bytes) in files {
        let target = files::destination(&store.skills_root, &relative)?;
        fs::create_dir_all(target.parent().context("技能路径缺少父目录")?)?;
        if !target.exists() {
            let mut file =
                tempfile::NamedTempFile::new_in(target.parent().context("技能路径缺少父目录")?)?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            file.persist_noclobber(&target)
                .context("无法原子写入技能文件，目标可能被其他进程创建")?;
        } else {
            ensure!(
                fs::read(&target)? == bytes,
                "技能版本内容变化，需要先卸载旧版本"
            );
        }
    }
    println!("已附带 CLI 技能：{}", store.skills_root.display());
    Ok(())
}
