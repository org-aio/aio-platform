use std::env;

use anyhow::{Result, bail};

const DEFAULT_API_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-5.5";

#[derive(Clone, Debug)]
pub(crate) struct AgentConfig {
    pub(crate) api_key: String,
    pub(crate) api_base: String,
    pub(crate) model: String,
}

pub(crate) fn from_env(model_names: &[&str]) -> Result<Option<AgentConfig>> {
    let Some(api_key) = first_env(&["OPENAI_API_KEY", "API_KEY"]) else {
        return Ok(None);
    };
    let api_base = first_env(&["OPENAI_BASE_URL", "OPENAI_BASEURL", "API_BASEURL"])
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let api_base = normalize_api_base(&api_base)?;
    let model = first_env(model_names).unwrap_or_else(|| DEFAULT_MODEL.to_owned());
    Ok(Some(AgentConfig {
        api_key,
        api_base,
        model,
    }))
}

fn first_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
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
