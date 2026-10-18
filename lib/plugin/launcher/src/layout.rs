//! 发布目录布局与 `current` 原子切换。
//!
//! ```text
//! <root>/releases/<revision>/   不可变发布物
//! <root>/current                指向当前发布目录的符号链接
//! <root>/switch.json            切换日志
//! <root>/.next                  切换前准备的临时链接
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 指定 revision 的不可变发布目录。
    pub fn release(&self, revision: &str) -> PathBuf {
        self.root.join("releases").join(revision)
    }

    pub fn current(&self) -> PathBuf {
        self.root.join("current")
    }

    pub fn journal(&self) -> PathBuf {
        self.root.join("switch.json")
    }

    fn next_link(&self) -> PathBuf {
        self.root.join(".next")
    }

    /// 读取 `current` 指向的真实目录；不存在返回 `None`。
    pub async fn current_target(&self) -> Result<Option<PathBuf>> {
        match tokio::fs::read_link(self.current()).await {
            Ok(target) => Ok(Some(target)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("读取当前发布链接失败: {}", self.current().display())),
        }
    }

    /// 原子切换到目标目录：先建临时链接，再用 rename 覆盖 `current`。
    pub async fn point_current(&self, target: &Path) -> Result<()> {
        let parent = self.root.as_path();
        tokio::fs::create_dir_all(self.root.join("releases"))
            .await
            .with_context(|| format!("创建发布目录失败: {}", self.root.display()))?;
        let next = self.next_link();
        let _ = tokio::fs::remove_file(&next).await;
        tokio::fs::symlink(target, &next)
            .await
            .with_context(|| format!("创建候选链接失败: {}", next.display()))?;
        tokio::fs::rename(&next, parent.join("current"))
            .await
            .with_context(|| format!("原子切换 current 失败: {}", target.display()))?;
        Ok(())
    }

    /// 校验发布目录位于本布局的 releases 之下，且不是符号链接。
    pub async fn ensure_managed(&self, directory: &Path) -> Result<()> {
        let releases = self.root.join("releases");
        let canonical_root = tokio::fs::canonicalize(&releases)
            .await
            .with_context(|| format!("发布根目录不存在: {}", releases.display()))?;
        let canonical = tokio::fs::canonicalize(directory)
            .await
            .with_context(|| format!("发布目录不存在: {}", directory.display()))?;
        let metadata = tokio::fs::symlink_metadata(directory)
            .await
            .with_context(|| format!("读取发布目录元数据失败: {}", directory.display()))?;
        ensure!(!metadata.file_type().is_symlink(), "发布目录不能是符号链接");
        ensure!(
            canonical.starts_with(&canonical_root),
            "发布目录必须位于 {} 之下",
            canonical_root.display()
        );
        Ok(())
    }
}
