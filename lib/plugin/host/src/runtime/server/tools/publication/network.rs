use anyhow::{Result, ensure};
use std::time::Duration;

pub(super) async fn bytes(url: &str, github: bool, maximum: usize) -> Result<Vec<u8>> {
    let mut request = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(url)
        .header("user-agent", "aio-cli-publication");
    if github {
        request = request.header("accept", "application/vnd.github+json");
        if let Ok(token) = std::env::var("AIO_DELIVERY_GITHUB_TOKEN") {
            request = request.bearer_auth(token);
        }
    }
    let mut response = request.send().await?;
    ensure!(
        response.status().is_success(),
        "发布资料暂时不可读取: HTTP {}",
        response.status()
    );
    let mut value = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(value.len() + chunk.len() <= maximum, "发布资料超过大小限制");
        value.extend_from_slice(&chunk);
    }
    Ok(value)
}
