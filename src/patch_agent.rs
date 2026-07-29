use std::env;

use crate::{CapabilityCatalog, ComponentCatalog, GraphPatchBatch, PatchOrigin, ProgramDefinition};
use anyhow::{Context, Result, bail};
use rig::providers::openai;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-5.2";

#[derive(Clone, Debug)]
pub struct ProgramPatchAgent {
    config: Option<AgentConfig>,
}

#[derive(Clone, Debug)]
struct AgentConfig {
    api_key: String,
    api_base: String,
    model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneratedProgramPatch {
    pub batch: GraphPatchBatch,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct PatchAgentResponse {
    batch: Value,
}

impl ProgramPatchAgent {
    pub fn from_env() -> Result<Self> {
        let Some(api_key) = first_env(["OPENAI_API_KEY", "API_KEY"]) else {
            return Ok(Self { config: None });
        };
        let api_base = first_env(["OPENAI_BASE_URL", "OPENAI_BASEURL", "API_BASEURL"])
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let api_base = normalize_api_base(&api_base)?;
        let model = first_env(["AZ_AIO_PROGRAM_AGENT_MODEL", "OPENAI_MODEL"])
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        Ok(Self {
            config: Some(AgentConfig {
                api_key,
                api_base,
                model,
            }),
        })
    }

    pub async fn generate(
        &self,
        prompt: &str,
        model_override: Option<&str>,
        base_version: i64,
        definition: &ProgramDefinition,
        components: &ComponentCatalog,
        capabilities: &CapabilityCatalog,
        previous_diagnostics: &[Value],
    ) -> Result<GeneratedProgramPatch> {
        let config = self
            .config
            .as_ref()
            .context("Vibe Agent 需要 OPENAI_API_KEY")?;
        let model = model_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&config.model);
        let client = openai::Client::builder()
            .api_key(&config.api_key)
            .base_url(&config.api_base)
            .build()
            .context("创建 ProgramPatchAgent Responses client 失败")?;
        let input = agent_input(
            prompt,
            base_version,
            definition,
            components,
            capabilities,
            previous_diagnostics,
        )?;
        let response = client
            .extractor::<PatchAgentResponse>(model)
            .preamble(agent_contract())
            .additional_params(json!({
                "store": false,
                "parallel_tool_calls": false,
                "reasoning": {"effort": "medium", "summary": "concise"}
            }))
            .max_tokens(8_192)
            .retries(2)
            .build()
            .extract_with_usage(&input)
            .await
            .with_context(|| format!("ProgramPatchAgent Responses 调用失败: {model}"))?;
        reject_forbidden_patch_keys(&response.data.batch)?;
        let mut batch = serde_json::from_value::<GraphPatchBatch>(response.data.batch)
            .context("ProgramPatchAgent 返回值不是 GraphPatchBatch")?;
        batch.base_version = base_version;
        batch.origin = PatchOrigin::Vibe;
        Ok(GeneratedProgramPatch {
            batch,
            model: model.to_owned(),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }
}

fn agent_input(
    prompt: &str,
    base_version: i64,
    definition: &ProgramDefinition,
    components: &ComponentCatalog,
    capabilities: &CapabilityCatalog,
    previous_diagnostics: &[Value],
) -> Result<String> {
    serde_json::to_string(&json!({
        "request": prompt,
        "base_version": base_version,
        "program": definition,
        "component_catalog": components,
        "capability_catalog": capabilities,
        "previous_diagnostics": previous_diagnostics,
    }))
    .context("序列化 ProgramPatchAgent 输入失败")
}

fn agent_contract() -> &'static str {
    r#"你是 AIO ProgramPatchAgent。你只能返回一个对象：{"batch": GraphPatchBatch JSON}。
不得返回或生成 Rust、SQL、HTML、CSS、JavaScript、Rhai、文件路径、URL 或解释文本。
只能使用输入中的稳定 SymbolId、组件 canonical_id、Capability canonical_id 和强类型 GraphPatch。
新声明必须分配合法 UUID。不得构造任意 URL、递归或无界循环；ForEach.max_items 必须在 1..=10000。
不要修改 base_version 和 origin，它们会由服务端覆盖。组件属性和事件必须存在于 component_catalog。
若 previous_diagnostics 非空，修复这些诊断并保留用户原始意图。"#
}

fn reject_forbidden_patch_keys(value: &Value) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "source"
                        | "source_text"
                        | "sql"
                        | "html"
                        | "css"
                        | "javascript"
                        | "script"
                        | "target_path"
                        | "file_path"
                ) {
                    bail!("ProgramPatchAgent 返回了禁止字段: {key}");
                }
                reject_forbidden_patch_keys(value)?;
            }
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                reject_forbidden_patch_keys(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn first_env<const N: usize>(names: [&str; N]) -> Option<String> {
    names.into_iter().find_map(|name| {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn normalize_api_base(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');
    if !value.starts_with("https://") && !value.starts_with("http://") {
        bail!("OPENAI_BASE_URL 必须是 HTTP(S) URL");
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_source_bearing_agent_output() {
        let value = json!({"batch": {"patches": [{"source_text": "fn main() {}"}]}});
        assert!(reject_forbidden_patch_keys(&value).is_err());
    }
}
