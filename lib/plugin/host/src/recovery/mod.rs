//! 部署配置的持久恢复记录；不依赖业务身份、数据库或已启动的插件。

mod journal;
mod storage;
#[cfg(test)]
mod tests;

pub use journal::{
    ConfigurationAttempt, ConfigurationJournal, ConfigurationRecord, ConfirmedConfiguration,
};
pub use storage::{configuration_digest, read_configuration};
