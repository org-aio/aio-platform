use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub code: u16,
    pub msg: String,
    pub data: Option<T>,
}

pub async fn get_api<T>(api_base_url: &str, path: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let url = api_url(api_base_url, path);
    let (status, text) = send_http("GET", &url, &[], None).await?;
    decode_api(status, &text, "GET", &url)
}

pub async fn post_api<I, O>(api_base_url: &str, path: &str, input: &I) -> Result<O, String>
where
    I: Serialize + ?Sized,
    O: DeserializeOwned,
{
    send_json("POST", api_base_url, path, input).await
}

pub async fn patch_api<I, O>(api_base_url: &str, path: &str, input: &I) -> Result<O, String>
where
    I: Serialize + ?Sized,
    O: DeserializeOwned,
{
    send_json("PATCH", api_base_url, path, input).await
}

pub async fn put_api<I, O>(api_base_url: &str, path: &str, input: &I) -> Result<O, String>
where
    I: Serialize + ?Sized,
    O: DeserializeOwned,
{
    send_json("PUT", api_base_url, path, input).await
}

pub async fn delete_api<O>(api_base_url: &str, path: &str) -> Result<O, String>
where
    O: DeserializeOwned,
{
    let url = api_url(api_base_url, path);
    let (status, text) = send_http("DELETE", &url, &[], None).await?;
    decode_api(status, &text, "DELETE", &url)
}

async fn send_json<I, O>(
    method: &str,
    api_base_url: &str,
    path: &str,
    input: &I,
) -> Result<O, String>
where
    I: Serialize + ?Sized,
    O: DeserializeOwned,
{
    let url = api_url(api_base_url, path);
    let body = serde_json::to_value(input)
        .map_err(|error| format!("{method} {url} 序列化失败: {error}"))?;
    let (status, text) = send_http(method, &url, &[], Some(&body)).await?;
    decode_api(status, &text, method, &url)
}

fn decode_api<T>(status: u16, text: &str, method: &str, url: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let payload = serde_json::from_str::<ApiResponse<T>>(text)
        .map_err(|error| format!("{method} {url} 返回无效 JSON: {error}"))?;
    if (200..300).contains(&status) && payload.code < 400 {
        payload
            .data
            .ok_or_else(|| format!("{method} {url} 返回空 data"))
    } else {
        Err(payload.msg)
    }
}

pub async fn send_http(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&Value>,
) -> Result<(u16, String), String> {
    send_http_platform(method, url, headers, body).await
}

#[cfg(target_arch = "wasm32")]
async fn send_http_platform(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&Value>,
) -> Result<(u16, String), String> {
    let mut request = match method {
        "GET" => gloo_net::http::Request::get(url),
        "POST" => gloo_net::http::Request::post(url),
        "PUT" => gloo_net::http::Request::put(url),
        "PATCH" => gloo_net::http::Request::patch(url),
        "DELETE" => gloo_net::http::Request::delete(url),
        _ => return Err(format!("不支持的 HTTP 方法: {method}")),
    };
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = if let Some(body) = body {
        request
            .json(body)
            .map_err(|error| format!("{method} {url} 请求体序列化失败: {error}"))?
            .send()
            .await
    } else {
        request.send().await
    }
    .map_err(|error| format!("{method} {url} 失败: {error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 {method} {url} 响应失败: {error}"))?;
    Ok((status, text))
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_http_platform(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&Value>,
) -> Result<(u16, String), String> {
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|error| format!("无效的 HTTP 方法: {error}"))?;
    let mut request = reqwest::Client::new().request(method.clone(), url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("{method} {url} 失败: {error}"))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取 {method} {url} 响应失败: {error}"))?;
    Ok((status, text))
}

pub fn api_url(base: &str, path: &str) -> String {
    let base = resolved_api_base(base);
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        path.to_owned()
    } else {
        format!("{base}{path}")
    }
}

#[cfg(target_arch = "wasm32")]
fn resolved_api_base(base: &str) -> String {
    base.to_owned()
}

#[cfg(not(target_arch = "wasm32"))]
fn resolved_api_base(base: &str) -> String {
    if !base.trim().is_empty() {
        return base.to_owned();
    }
    std::env::var("AIO_API_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned())
}

#[cfg(target_arch = "wasm32")]
pub async fn write_clipboard(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "无法访问浏览器剪贴板".to_string())?;
    wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(text))
        .await
        .map(|_| ())
        .map_err(|_| "复制失败，请检查浏览器剪贴板权限".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn write_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(|error| format!("写入系统剪贴板失败: {error}"))
}

pub async fn sleep_ms(milliseconds: u32) {
    sleep_ms_platform(milliseconds).await;
}

#[cfg(target_arch = "wasm32")]
async fn sleep_ms_platform(milliseconds: u32) {
    gloo_timers::future::TimeoutFuture::new(milliseconds).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn sleep_ms_platform(milliseconds: u32) {
    tokio::time::sleep(std::time::Duration::from_millis(u64::from(milliseconds))).await;
}

pub fn format_unix_timestamp(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let value = value.strip_prefix("unix:").unwrap_or(value);
    let Ok(seconds) = value.parse::<i64>() else {
        return value.to_string();
    };
    format_timestamp(seconds.saturating_mul(1_000))
}

pub fn format_millis_timestamp(value: i64) -> String {
    if value <= 0 {
        return String::new();
    }
    format_timestamp(value)
}

#[cfg(target_arch = "wasm32")]
fn format_timestamp(milliseconds: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(milliseconds as f64));
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        date.get_full_year(),
        date.get_month().saturating_add(1),
        date.get_date(),
        date.get_hours(),
        date.get_minutes(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn format_timestamp(milliseconds: i64) -> String {
    use chrono::{DateTime, Local};

    DateTime::from_timestamp_millis(milliseconds)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| milliseconds.to_string())
}

#[cfg(test)]
mod tests {
    use super::api_url;

    #[test]
    fn explicit_api_base_is_normalized() {
        assert_eq!(
            api_url("http://127.0.0.1:9000/", "/api/runtime/program/image"),
            "http://127.0.0.1:9000/api/runtime/program/image"
        );
    }
}
