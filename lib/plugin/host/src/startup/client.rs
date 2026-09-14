use gloo_net::http::Request;

use super::{ApplicationSnapshot, LoadedApplication};
use crate::runtime::RuntimeResponse;

pub(crate) async fn load(previous: Option<LoadedApplication>) -> Result<LoadedApplication, String> {
    if previous.is_none()
        && let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("aio-startup-snapshot"))
    {
        let text = element.text_content().unwrap_or_default();
        element.remove();
        if let Ok(snapshot) = serde_json::from_str(&text) {
            return Ok(snapshot);
        }
    }
    for attempt in 0..2 {
        let signal = web_sys::AbortSignal::timeout_with_u32(8_000);
        let mut request = Request::get("/api/runtime/bootstrap").abort_signal(Some(&signal));
        if let Some(etag) = previous.as_ref().and_then(|value| value.etag.as_deref()) {
            request = request.header("if-none-match", etag);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) if attempt == 0 => continue,
            Err(_) => return Err("连接工作区超时或网络不可用".to_owned()),
        };
        if response.status() == 304 {
            return previous.ok_or_else(|| "工作区返回了无效的缓存响应".to_owned());
        }
        if !response.ok() {
            if response.status() >= 500 && attempt == 0 {
                continue;
            }
            return Err(format!("加载工作区失败 (HTTP {})", response.status()));
        }
        let etag = response.headers().get("etag");
        match response
            .json::<RuntimeResponse<Option<ApplicationSnapshot>>>()
            .await
        {
            Ok(response) => {
                return Ok(LoadedApplication {
                    snapshot: response.data,
                    etag,
                });
            }
            Err(_) if attempt == 0 => continue,
            Err(_) => return Err("工作区响应未完整接收，请重试".to_owned()),
        }
    }
    unreachable!()
}
