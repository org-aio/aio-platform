use serde::{Deserialize, Serialize};

/// 传输请求与脚本共用的原子版本切换条件，不允许从输入选择租户或操作人。
#[derive(Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub revision: Option<String>,
    pub expected_generation: i64,
}

/// 当前活动版本及乐观提交代数；None 表示租户已停用或尚未激活增强。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Head {
    pub revision: Option<String>,
    pub generation: i64,
}

/// 不可变候选的列表条目，不在目录中复制源码。
#[derive(Clone, Deserialize, Serialize)]
pub struct Revision {
    pub revision: String,
    pub author: String,
    pub created_at: i64,
}

/// 版本指针与此事件在同一事务提交；时间统一使用毫秒。
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Event {
    pub generation: i64,
    pub revision: Option<String>,
    pub actor: String,
    pub action: String,
    pub created_at: i64,
}

/// 只记录计数，不记录调用输入、输出和凭据；未完成计数也包含取消与进程中断。
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct Observation {
    pub path: String,
    pub started: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub updated_at: i64,
}
