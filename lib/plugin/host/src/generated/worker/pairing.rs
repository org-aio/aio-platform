use dioxus::prelude::*;

use super::{model::Worker, view::decode_response};

pub(super) const UNAVAILABLE: &str = "这个一次性配对链接已使用或失效，不影响已配对设备。连接新设备时，请在那台设备上重新运行 aio-space connect。";

/// 结束一次配对入口，保留页面参数、锚点和路由状态，刷新不再重放旧配对码。
pub(super) fn finish(mut code: Signal<Option<String>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("无法访问当前页面")?;
    let clear = || {
        let url = web_sys::Url::new(&window.location().href()?)?;
        url.search_params().delete("worker_pair");
        let history = window.history()?;
        history.replace_state_with_url(
            &history.state()?,
            "",
            Some(&format!("{}{}{}", url.pathname(), url.search(), url.hash())),
        )
    };
    clear().map_err(|error| format!("清理配对链接失败：{error:?}"))?;
    code.set(None);
    Ok(())
}

/// 无效、已消费或过期的入口不代表已授权设备失效；其他请求错误仍须展示。
pub(super) async fn lookup(code: &str) -> Result<Option<Worker>, String> {
    let response = gloo_net::http::Request::get(&format!("/api/runtime/workers/pairings/{code}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == 400 {
        check_unavailable(response).await?;
        return Ok(None);
    }
    decode_response(response).await.map(Some)
}

/// 打开确认页后，配对码也可能被另一页面消费或到期。
pub(super) async fn approve(code: &str) -> Result<bool, String> {
    let response = gloo_net::http::Request::post(&format!("/api/runtime/workers/pairings/{code}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == 400 {
        check_unavailable(response).await?;
        return Ok(false);
    }
    decode_response::<()>(response).await?;
    Ok(true)
}

/// 设备数量上限、数据库故障等也可能返回 400，不能据此丢弃仍有效的配对入口。
async fn check_unavailable(response: gloo_net::http::Response) -> Result<(), String> {
    let body = response.text().await.map_err(|error| error.to_string())?;
    let value = serde_json::from_str::<serde_json::Value>(&body).ok();
    match value.as_ref().and_then(|value| value["error"].as_str()) {
        Some("配对码无效或已过期" | "配对码无效、已使用或已过期") => Ok(()),
        _ => Err(format!("请求失败（HTTP 400）：{body}")),
    }
}
