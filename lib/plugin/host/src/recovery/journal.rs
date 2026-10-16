use std::{
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::storage;

/// 最后一次完成初始化或在线提交的配置；调用方仍需按当前二进制契约重新校验。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedConfiguration {
    pub digest: String,
    pub value: Value,
}

/// 准备记录先落盘，未确认的尝试在下次启动时自动隔离。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationAttempt {
    id: Uuid,
    pub digest: String,
    rejected: bool,
}

/// 可在业务插件不可用时读取的运维快照；不表示当前进程健康状态。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationRecord {
    schema_version: u32,
    pub source: PathBuf,
    pub confirmed: Option<ConfirmedConfiguration>,
    pub attempt: Option<ConfigurationAttempt>,
}

/// 唯一写入者持有操作系统文件锁；记录成功前不得把候选当作可恢复版本。
pub struct ConfigurationJournal {
    // 文件句柄的生命周期就是本次宿主的所有权，不能通过删除锁文件夺取。
    _lock: File,
    path: PathBuf,
    record: ConfigurationRecord,
    healthy: bool,
}

impl ConfigurationJournal {
    pub fn open(directory: &Path, source: &Path) -> Result<Self> {
        let (path, source) = storage::paths(directory, source)?;
        let lock = storage::lock(directory)?;
        let record = load(&path, &source)?.unwrap_or(ConfigurationRecord {
            schema_version: 1,
            source,
            confirmed: None,
            attempt: None,
        });
        Ok(Self {
            _lock: lock,
            path,
            record,
            healthy: true,
        })
    }

    /// 只读状态不创建记录、不争夺锁；可以在服务运行时使用。
    pub fn inspect(directory: &Path, source: &Path) -> Result<Option<ConfigurationRecord>> {
        let (path, source) = storage::paths(directory, source)?;
        load(&path, &source)
    }

    pub fn confirmed(&self) -> Option<&ConfirmedConfiguration> {
        self.record.confirmed.as_ref()
    }

    pub fn blocked(&self, digest: &str) -> bool {
        self.record
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.digest == digest)
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy
    }

    /// 在解析候选或创建其资源之前记录尝试；返回令牌只能确认本次操作。
    pub fn begin(&mut self, digest: String) -> Result<ConfigurationAttempt> {
        ensure!(
            !self.blocked(&digest),
            "该配置上次未完成或已被拒绝，需要修改配置或显式重试"
        );
        validate_digest(&digest)?;
        let attempt = ConfigurationAttempt {
            id: Uuid::new_v4(),
            digest,
            rejected: false,
        };
        let mut next = self.record.clone();
        next.attempt = Some(attempt.clone());
        self.save(next)?;
        Ok(attempt)
    }

    /// 只有资源、路由及监听准备成功后确认；写入失败后调用方必须停止继续变更。
    pub fn confirm(&mut self, attempt: &ConfigurationAttempt, value: Value) -> Result<()> {
        self.check_attempt(attempt)?;
        let mut next = self.record.clone();
        next.confirmed = Some(ConfirmedConfiguration {
            digest: attempt.digest.clone(),
            value,
        });
        next.attempt = None;
        self.save(next)
    }

    pub fn reject(&mut self, attempt: &ConfigurationAttempt) -> Result<()> {
        self.check_attempt(attempt)?;
        let mut next = self.record.clone();
        let mut rejected = attempt.clone();
        rejected.rejected = true;
        next.attempt = Some(rejected);
        self.save(next)
    }

    /// 仅供持有独占锁的本机运维命令使用，允许下一次启动重新尝试当前配置。
    pub fn allow_retry(&mut self) -> Result<()> {
        let mut next = self.record.clone();
        next.attempt = None;
        self.save(next)
    }

    /// 调用方校验最后成功配置后，显式恢复部署文件；从不默认恢复为全量启用。
    pub fn restore(&mut self) -> Result<()> {
        ensure!(self.healthy, "恢复记录写入状态不确定，请退出并重新打开");
        let confirmed = self
            .record
            .confirmed
            .as_ref()
            .context("没有已确认的可恢复配置")?;
        let bytes = serde_json::to_vec_pretty(&confirmed.value)?;
        if let Err(error) = storage::write_atomic(&self.record.source, &bytes) {
            self.healthy = false;
            return Err(error.context("恢复部署文件失败"));
        }
        let mut next = self.record.clone();
        next.confirmed = Some(ConfirmedConfiguration {
            digest: storage::configuration_digest(&bytes),
            value: confirmed.value.clone(),
        });
        next.attempt = None;
        self.save(next)
    }

    fn check_attempt(&self, attempt: &ConfigurationAttempt) -> Result<()> {
        ensure!(
            self.record.attempt.as_ref() == Some(attempt) && !attempt.rejected,
            "恢复提交令牌已失效"
        );
        Ok(())
    }

    fn save(&mut self, next: ConfigurationRecord) -> Result<()> {
        ensure!(self.healthy, "恢复记录写入状态不确定，请退出并重新打开");
        let written = (|| {
            let bytes = serde_json::to_vec_pretty(&next)?;
            ensure!(
                bytes.len() as u64 <= storage::MAX_RECORD_BYTES,
                "恢复记录超过大小限制"
            );
            storage::write_atomic(&self.path, &bytes)
        })();
        if let Err(error) = written {
            self.healthy = false;
            return Err(error.context("持久化运行时恢复记录失败"));
        }
        self.record = next;
        Ok(())
    }
}

fn load(path: &Path, source: &Path) -> Result<Option<ConfigurationRecord>> {
    let bytes = match storage::read(path, storage::MAX_RECORD_BYTES) {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.context("读取运行时恢复记录失败")),
    };
    let record: ConfigurationRecord =
        serde_json::from_slice(&bytes).context("运行时恢复记录损坏")?;
    ensure!(
        record.schema_version == 1 && record.source == source,
        "恢复记录版本或部署来源不匹配"
    );
    if let Some(confirmed) = &record.confirmed {
        validate_digest(&confirmed.digest)?;
    }
    if let Some(attempt) = &record.attempt {
        validate_digest(&attempt.digest)?;
    }
    Ok(Some(record))
}

fn validate_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "恢复内容摘要无效"
    );
    Ok(())
}
