//! 可选计费服务独立于认证；未安装时用量上报返回未接入计费。

use serde::{Deserialize, Serialize};
use std::any::Any;

#[async_trait::async_trait]
pub trait MeteringProvider: Any + Send + Sync {
    async fn meter(
        &self,
        tenant: &str,
        user: &str,
        source: &str,
        resource: &str,
        quantity: i64,
        idempotency_key: &str,
    ) -> anyhow::Result<MeterOutcome>;
}

/// 一次用量上报的结算结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterOutcome {
    pub amount_micros: i64,
    pub grant_consumed: i64,
    pub balance_charged_micros: i64,
    pub balance_after_micros: i64,
    pub duplicate: bool,
}
