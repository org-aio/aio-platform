use crate::ToolManifest;
use anyhow::{Context as _, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Installation {
    pub manifest: ToolManifest,
    pub state: String,
    pub completed_steps: usize,
}

pub struct Store {
    root: PathBuf,
}

pub fn data_root() -> Result<PathBuf> {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let root = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"));
    Ok(root.context("无法定位当前用户的数据目录")?.join("aio"))
}

impl Store {
    pub fn user() -> Result<Self> {
        Self::new(data_root()?.join("tools"))
    }
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { root })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        ensure!(crate::validation::identifier(id), "工具 ID 无效");
        Ok(self.root.join(format!("{id}.json")))
    }
    pub(super) fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        file.try_lock_exclusive()
            .context("另一个工具操作正在执行，请稍后重试")?;
        Ok(file)
    }
    pub fn read(&self, id: &str) -> Result<Option<Installation>> {
        let path = self.path(id)?;
        match fs::read(path) {
            Ok(bytes) => {
                let record: Installation = serde_json::from_slice(&bytes)?;
                record.manifest.validate()?;
                ensure!(record.manifest.id == id, "安装记录 ID 不匹配");
                Ok(Some(record))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    pub fn list(&self) -> Result<Vec<Installation>> {
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Some(record) = self.read(
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .context("安装记录名称无效")?,
                )?
            {
                result.push(record);
            }
        }
        Ok(result)
    }
    pub(super) fn save(&self, record: &Installation) -> Result<()> {
        let target = self.path(&record.manifest.id)?;
        atomic_write(&target, &serde_json::to_vec_pretty(record)?)
    }
    pub(super) fn remove(&self, id: &str) -> Result<()> {
        fs::remove_file(self.path(id)?)?;
        Ok(())
    }
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = target.with_extension("tmp");
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temp, target)?;
    Ok(())
}

pub(crate) fn helper_root() -> Result<PathBuf> {
    Ok(data_root()?.join("helper"))
}
