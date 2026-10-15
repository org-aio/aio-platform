use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 插件通过版本化能力路径调用宿主增强；身份始终由宿主补充。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnhancementRequest {
    pub path: String,
    pub input: Value,
}

/// 返回执行所用的不可变版本，便于插件记录和重放结果。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnhancementResponse {
    pub revision: String,
    pub output: Value,
}

/// 路径是跨进程能力契约，不承担进程内插件的类型身份。
pub fn validate_enhancement_path(path: &str) -> Result<()> {
    let segments: Vec<_> = path.split('/').collect();
    ensure!(
        path.len() <= 128
            && segments.len() >= 3
            && segments[0].is_empty()
            && segments[1].strip_prefix('v').is_some_and(|version| {
                !version.is_empty()
                    && !version.starts_with('0')
                    && version.bytes().all(|c| c.is_ascii_digit())
            })
            && segments[2..].iter().all(|segment| {
                !segment.is_empty()
                    && segment
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            }),
        "增强能力必须使用 /v1/领域/操作 形式的版本化路径"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_paths_are_versioned_exact_addresses() -> Result<()> {
        validate_enhancement_path("/v1/text/normalize")?;
        for invalid in [
            "/text/normalize",
            "/v0/text",
            "/v01/text",
            "/v1/*",
            "/v1/../text",
            "/v1/text/",
            "/v1/Text",
            "/v1/text?admin=true",
        ] {
            assert!(validate_enhancement_path(invalid).is_err(), "{invalid}");
        }
        Ok(())
    }

    #[test]
    fn empty_enhancements_preserve_existing_bundle_serialization() -> Result<()> {
        let grants = serde_json::to_value(crate::CapabilityGrants::default())?;
        assert!(grants.get("enhancements").is_none());
        let restored: crate::CapabilityGrants = serde_json::from_value(grants)?;
        assert!(restored.enhancements.is_empty());
        Ok(())
    }
}
