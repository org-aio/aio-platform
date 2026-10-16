use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一次整体发布的增强集合；同一版本中的路径不会分批切换。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub schema_version: u32,
    pub enhancements: Vec<Enhancement>,
}

/// 一个稳定能力契约及其候选实现；路径版本变化表示破坏性契约变化。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Enhancement {
    pub path: String,
    pub language: Language,
    pub source: String,
    pub examples: Vec<Example>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rhai,
    TypeScript,
}

/// 激活前必须通过的契约样例；升级还要通过旧版已有样例。
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub input: Value,
    pub output: Value,
}

/// 宿主注入的调用归属；脚本不接收会话凭据、数据库连接或网络句柄。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptContext {
    pub tenant_id: String,
    pub source_id: String,
}
