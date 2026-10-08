use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

/// 路由不包含凭据，只允许原版应用内部路径。
pub(super) fn route(value: &str) -> Result<()> {
    ensure!(
        value.len() <= 2048
            && value.starts_with('/')
            && !value.starts_with("//")
            && !value.contains(['\\', '\0', '\r', '\n', '#', '?']),
        "视图路由无效"
    );
    let url = reqwest::Url::parse(&format!("https://view.invalid{value}"))?;
    ensure!(
        url.host_str() == Some("view.invalid")
            && url.username().is_empty()
            && url.password().is_none(),
        "视图路由来源无效"
    );
    Ok(())
}

pub(super) fn asset_path(value: &str) -> Result<()> {
    ensure!(
        value.len() <= 1024
            && !value.is_empty()
            && !value.contains(['\\', '\0', '?', '#'])
            && value
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "资源路径无效"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cross_origin_routes_and_asset_traversal() {
        for value in [
            "//evil.invalid/x",
            "/\\evil.invalid",
            "https://evil.invalid",
            "/x\n",
            "/local/chat?token=secret",
        ] {
            assert!(route(value).is_err());
        }
        for value in [
            "../config",
            "assets/../config",
            "/config",
            "assets\\config",
            "assets/x?token=1",
        ] {
            assert!(asset_path(value).is_err());
        }
        assert!(route("/local/chat-123").is_ok());
        assert!(asset_path("assets/index-123.js").is_ok());
    }
}
