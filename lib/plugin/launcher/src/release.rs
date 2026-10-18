//! 发布物描述与候选校验；启动器只信任清单中显式列出的相对路径与摘要。

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 发布目录顶部清单文件名；与宿主 aio.toml 分离，避免业务配置影响切换判定。
pub const RELEASE_MANIFEST: &str = "release-manifest.json";

/// 一次发布的不可变描述。启动器在执行前逐项复核摘要与路径。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub revision: String,
    pub host_binary: String,
    pub host_sha256: String,
    pub web_index: String,
    #[serde(default)]
    pub extra_files: BTreeMap<String, String>,
}

impl ReleaseManifest {
    /// 读取并解析候选清单；未知字段直接拒绝，避免静默忽略新协议。
    pub async fn read(directory: &Path) -> Result<Self> {
        let path = directory.join(RELEASE_MANIFEST);
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("读取发布清单失败: {}", path.display()))?;
        let manifest: Self =
            serde_json::from_slice(&bytes).context("发布清单不是合法 JSON 或包含未知字段")?;
        manifest.validate_shape()?;
        Ok(manifest)
    }

    fn validate_shape(&self) -> Result<()> {
        ensure!(
            self.revision.len() == 40 && self.revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "发布 revision 必须是 40 位十六进制"
        );
        ensure!(
            self.host_sha256.len() == 64
                && self
                    .host_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "宿主二进制摘要必须是 64 位十六进制"
        );
        Ok(())
    }

    /// 复核清单声明的每个文件都位于候选目录内且摘要一致。
    pub async fn verify(&self, directory: &Path) -> Result<()> {
        self.validate_shape()?;
        for (relative, expected) in self.declared_files() {
            let path = safe_join(directory, &relative)?;
            let bytes = tokio::fs::read(&path)
                .await
                .with_context(|| format!("读取发布文件失败: {}", path.display()))?;
            let actual = format!("{:x}", Sha256::digest(&bytes));
            ensure!(
                actual == *expected,
                "发布文件摘要不匹配: {relative}，期望 {expected}，实际 {actual}"
            );
        }
        Ok(())
    }

    /// 为不包含清单的既有发布生成描述。
    ///
    /// 这是协议迁移的一次性步骤：只有部署侧显式调用 `adopt` 才会写入，之后
    /// `boot` 对每个发布都保持严格校验，不会出现“缺清单也放行”的弱化路径。
    pub async fn adopt(directory: &Path, revision: &str) -> Result<Self> {
        ensure!(
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "发布 revision 必须是 40 位十六进制"
        );
        let host_binary = "aio-idea".to_owned();
        let web_index = "web/index.html".to_owned();
        let mut extra_files = BTreeMap::new();
        for relative in [&web_index, &"aio.toml".to_owned()] {
            extra_files.insert(relative.clone(), digest_file(directory, relative).await?);
        }
        Ok(Self {
            revision: revision.to_owned(),
            host_sha256: digest_file(directory, &host_binary).await?,
            host_binary,
            web_index,
            extra_files,
        })
    }

    /// 声明文件按路径排序，便于稳定的错误顺序与测试断言。
    pub fn declared_files(&self) -> BTreeMap<String, String> {
        let mut files = self.extra_files.clone();
        files.insert(self.host_binary.clone(), self.host_sha256.clone());
        files
    }
}

/// 拒绝绝对路径、父目录跳转与 Windows 前缀，确保候选不能逃逸发布目录。
pub fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let candidate = Path::new(relative);
    ensure!(!relative.is_empty(), "发布路径不能为空");
    ensure!(
        candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_))),
        "发布路径必须是普通相对路径: {relative}"
    );
    Ok(root.join(candidate))
}

/// 计算发布目录内一个相对路径的 SHA-256。
async fn digest_file(directory: &Path, relative: &str) -> Result<String> {
    let path = safe_join(directory, relative)?;
    let bytes = tokio::fs::read(&path)
        .await
        .with_context(|| format!("读取发布文件失败: {}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}
