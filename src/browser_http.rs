use gloo_net::http::Request;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|error| format!("GET {url} 失败: {error}"))?;
    decode_api(response, "GET", &url).await
}

pub async fn post_api<I, O>(api_base_url: &str, path: &str, input: &I) -> Result<O, String>
where
    I: Serialize + ?Sized,
    O: DeserializeOwned,
{
    send_json(Request::post, "POST", api_base_url, path, input).await
}

pub async fn patch_api<I, O>(api_base_url: &str, path: &str, input: &I) -> Result<O, String>
where
    I: Serialize + ?Sized,
    O: DeserializeOwned,
{
    send_json(Request::patch, "PATCH", api_base_url, path, input).await
}

pub async fn delete_api<O>(api_base_url: &str, path: &str) -> Result<O, String>
where
    O: DeserializeOwned,
{
    let url = api_url(api_base_url, path);
    let response = Request::delete(&url)
        .send()
        .await
        .map_err(|error| format!("DELETE {url} 失败: {error}"))?;
    decode_api(response, "DELETE", &url).await
}

async fn send_json<I, O>(
    request: fn(&str) -> gloo_net::http::RequestBuilder,
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
    let builder = request(&url)
        .json(input)
        .map_err(|error| format!("{method} {url} 序列化失败: {error}"))?;
    let response = builder
        .send()
        .await
        .map_err(|error| format!("{method} {url} 失败: {error}"))?;
    decode_api(response, method, &url).await
}

async fn decode_api<T>(
    response: gloo_net::http::Response,
    method: &str,
    url: &str,
) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let status = response.status();
    let payload = response
        .json::<ApiResponse<T>>()
        .await
        .map_err(|error| format!("{method} {url} 返回无效 JSON: {error}"))?;
    if (200..300).contains(&status) && payload.code < 400 {
        payload
            .data
            .ok_or_else(|| format!("{method} {url} 返回空 data"))
    } else {
        Err(payload.msg)
    }
}

pub fn api_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        path.to_owned()
    } else {
        format!("{base}{path}")
    }
}
