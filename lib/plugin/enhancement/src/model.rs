use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一次整体发布的增强集合；同一版本中的路径不会分批切换。
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub schema_version: u32,
    pub enhancements: Vec<Enhancement>,
}

/// 一个稳定能力契约及其候选实现；路径版本变化表示破坏性契约变化。
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Enhancement {
    pub path: String,
    pub language: Language,
    pub source: String,
    /// 输入契约由公共执行链校验；省略表示任意 JSON，不从 TypeScript 注解推断。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<schemars::Schema>,
    /// 输出契约同样约束嵌套调用和替换执行器，不能通过切换语言绕过。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<schemars::Schema>,
    /// 可以组合的同版本增强路径；实际执行还需调用者逐项授权。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
    /// 可调用的宿主能力路径；发布声明与调用授权分别校验。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub examples: Vec<Example>,
}

/// 面向插件及协议的发现结果，不包含实现源码或候选样例。
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct EnhancementContract {
    pub path: String,
    pub input_schema: schemars::Schema,
    pub output_schema: schemars::Schema,
}

impl Enhancement {
    /// 省略的 schema 在发现时明确表现为 true，持久化原包仍保留原始字段形态。
    pub fn contract(&self) -> EnhancementContract {
        EnhancementContract {
            path: self.path.clone(),
            input_schema: self.input_schema.clone().unwrap_or_else(|| true.into()),
            output_schema: self.output_schema.clone().unwrap_or_else(|| true.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rhai,
    TypeScript,
}

/// 激活前必须通过的契约样例；升级还要通过旧版已有样例。
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub input: Value,
    pub output: Value,
    /// 候选验证只消费这些调用记录，不执行真实宿主能力。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<crate::CapabilityExample>,
}

/// 宿主注入的调用归属；脚本不接收会话凭据、数据库连接或网络句柄。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptContext {
    pub tenant_id: String,
    pub source_id: String,
}
