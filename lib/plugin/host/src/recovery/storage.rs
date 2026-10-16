use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

pub(super) const MAX_RECORD_BYTES: u64 = 256 * 1024;
const MAX_CONFIGURATION_BYTES: u64 = 64 * 1024;

/// 有界读取部署方配置，不跟随流设备持续等待；调用方负责解析业务契约。
pub fn read_configuration(path: &Path) -> Result<Vec<u8>> {
    read(path, MAX_CONFIGURATION_BYTES)
}

/// 内容摘要只表示一次部署输入，不作为原生插件身份。
pub fn configuration_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn read(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    ensure!(
        std::fs::metadata(path)?.is_file(),
        "配置和恢复记录必须是普通文件"
    );
    let file = File::open(path).context("打开配置或恢复记录失败")?;
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= maximum, "配置或恢复记录超过大小限制");
    Ok(bytes)
}

pub(super) fn lock(directory: &Path) -> Result<File> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(directory)
        .context("创建运行时恢复目录失败")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(directory.join("owner.lock"))
        .context("打开恢复记录锁失败")?;
    file.try_lock()
        .context("恢复记录已被宿主或另一个恢复进程占用")?;
    Ok(file)
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().context("恢复文件必须有父目录")?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).context("创建恢复临时文件失败")?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all().context("刷新恢复文件失败")?;
    temporary.persist(path).context("原子替换恢复文件失败")?;
    #[cfg(unix)]
    File::open(directory)?
        .sync_all()
        .context("刷新恢复目录失败")?;
    Ok(())
}

pub(super) fn paths(directory: &Path, source: &Path) -> Result<(PathBuf, PathBuf)> {
    let directory = std::path::absolute(directory)?;
    let source = std::path::absolute(source)?;
    ensure!(
        !source.starts_with(&directory),
        "部署配置不能放在恢复记录目录内"
    );
    Ok((directory.join("record.json"), source))
}
