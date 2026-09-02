use crate::{
    CapabilityCatalog, GraphPatch, GraphPatchBatch, PatchOrigin, ProgramDefinition, agent_config,
};
use anyhow::{Context, Result, bail};
use rig::providers::openai;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default)]
pub struct ProgramPatchAgent {
    config: Option<agent_config::AgentConfig>,
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
    patches: Vec<Value>,
}

impl ProgramPatchAgent {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            config: agent_config::from_env(&["AZ_AIO_PROGRAM_AGENT_MODEL", "OPENAI_MODEL"])?,
        })
    }

    pub async fn generate(
        &self,
        prompt: &str,
        model_override: Option<&str>,
        base_version: i64,
        definition: &ProgramDefinition,
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
        let patches = parse_agent_patches(response.data.patches)?;
        let batch = GraphPatchBatch {
            base_version,
            patches,
            origin: PatchOrigin::Vibe,
        };
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
    capabilities: &CapabilityCatalog,
    previous_diagnostics: &[Value],
) -> Result<String> {
    serde_json::to_string(&json!({
        "request": prompt,
        "base_version": base_version,
        "program": definition,
        "capability_catalog": capabilities,
        "previous_diagnostics": previous_diagnostics,
    }))
    .context("序列化 ProgramPatchAgent 输入失败")
}

fn agent_contract() -> &'static str {
    r#"你是 AIO ProgramPatchAgent。你只能返回一个对象：{"patches": GraphPatch JSON 数组}。
不得返回或生成 Rust、SQL、HTML、CSS、JavaScript、Rhai、文件路径、外部 URL 或解释文本。
只能使用输入中的稳定 SymbolId、页面渲染声明、模型字段、Capability canonical_id 和强类型 GraphPatch。
新声明必须分配合法 UUID。PageEndpointDefinition 只能使用本应用以 / 开头的相对 REST 路径，
并完整声明 method、inputs 和 outputs；title 是可省略的中文显示名，不得声明额外接口标识或保存生成需求；
不得构造外部 URL、递归或无界循环；ForEach.max_items 必须在 1..=10000。
页面接口插入格式固定为：{"kind":"insert","parent_id":"页面 UUID","collection":"page_endpoints","index":0,
"entity":{"kind":"page_endpoint","value":{"id":"新 UUID","title":"可选中文显示名",
"method":"GET|POST|PUT|PATCH|DELETE","path":"/相对路径","inputs":[{"id":"新 UUID","name":"snake_case",
"title":"中文说明","location":"path|query|header|body","value_type":{"kind":"text"},"required":true}],
"outputs":[{"id":"新 UUID","name":"snake_case","title":"中文说明","value_type":{"kind":"text"}}]}}}。
模型主键只能通过 SetProperty 的 model_primary_key 修改，value 为
{"generation":"uuid"} 或 {"generation":"auto_increment"}；不得把 id 插入普通字段集合。
不要返回 base_version 和 origin，它们由服务端填写。页面只能选择 convention_file、tree_table 或 crud_table。
若 previous_diagnostics 非空，修复这些诊断并保留用户原始意图。"#
}

fn parse_agent_patches(values: Vec<Value>) -> Result<Vec<GraphPatch>> {
    reject_forbidden_patch_keys(&Value::Array(values.clone()))?;
    serde_json::from_value(Value::Array(values))
        .context("ProgramPatchAgent 返回值不是 GraphPatch 数组")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SymbolId;

    #[test]
    fn rejects_source_bearing_agent_output() {
        let value = json!({"patches": [{"source_text": "fn main() {}"}]});
        assert!(reject_forbidden_patch_keys(&value).is_err());
    }

    #[test]
    fn builds_batch_from_agent_patches_without_transport_fields() {
        let target_id = SymbolId::new();
        let result = parse_agent_patches(vec![json!({
            "kind": "delete",
            "target_id": target_id,
        })]);
        let patches = match result {
            Ok(patches) => patches,
            Err(error) => panic!("合法 GraphPatch 应当能够解析: {error:#}"),
        };
        let batch = GraphPatchBatch {
            base_version: 42,
            patches,
            origin: PatchOrigin::Vibe,
        };

        assert_eq!(batch.base_version, 42);
        assert_eq!(batch.origin, PatchOrigin::Vibe);
        assert_eq!(batch.patches, vec![GraphPatch::Delete { target_id }],);
    }

    #[test]
    fn reports_invalid_patch_shape_with_serde_cause() {
        let result = parse_agent_patches(vec![json!({"kind": "delete"})]);
        let error = match result {
            Ok(_) => panic!("缺少 target_id 的 GraphPatch 必须失败"),
            Err(error) => error,
        };

        assert!(format!("{error:#}").contains("missing field `target_id`"));
    }
}
