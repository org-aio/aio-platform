use az_plugin_runtime::bindings::aio::plugin::{metadata, transport};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Description {
    #[serde(default)]
    pub process: bool,
    pub label: String,
    pub pages: Vec<Page>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Page {
    pub id: String,
    pub label: String,
    pub entry: String,
    pub scene: Option<(String, String)>,
    pub menu_path: Vec<String>,
    pub permission: Option<String>,
    pub surface: String,
}

impl From<metadata::Description> for Description {
    fn from(value: metadata::Description) -> Self {
        Self {
            process: false,
            label: value.label,
            pages: value
                .pages
                .into_iter()
                .map(|p| Page {
                    id: p.id,
                    label: p.label,
                    entry: p.entry,
                    scene: p.scene.map(|s| (s.id, s.label)),
                    menu_path: p.menu_path,
                    permission: p.permission,
                    surface: match p.surface {
                        metadata::Surface::Workspace => "workspace",
                        metadata::Surface::Fullscreen => "fullscreen",
                        metadata::Surface::AccountEntry => "account-entry",
                        metadata::Surface::AccountMenu => "account-menu",
                    }
                    .into(),
                })
                .collect(),
        }
    }
}

impl Description {
    pub(super) fn with_settings(
        mut self,
        bundle: &az_plugin_bundle::VerifiedBundle,
    ) -> anyhow::Result<Self> {
        if let Some(id) = &bundle.manifest().plugin.settings_page {
            use anyhow::Context;
            let page = self
                .pages
                .iter_mut()
                .find(|page| &page.id == id)
                .context("清单中的设置页面不存在")?;
            anyhow::ensure!(
                page.surface == "fullscreen" && page.scene.is_none() && page.menu_path.is_empty(),
                "设置页必须声明为独立全屏页面，不能进入业务导航"
            );
            page.surface = "settings".into();
        }
        Ok(self)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    #[serde(default)]
    pub headers: Vec<Header>,
    /// 正文按 base64 传输：JSON 里的字节数组会放大到约 3.4 倍，浏览器还要解析一座数字山。
    #[serde(default, with = "base64_body")]
    pub body: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    pub name: String,
    pub value: String,
}

impl TryFrom<Request> for transport::Request {
    type Error = anyhow::Error;
    fn try_from(r: Request) -> Result<Self, Self::Error> {
        anyhow::ensure!(
            ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&r.method.as_str()),
            "请求方法无效"
        );
        anyhow::ensure!(
            r.path.starts_with('/')
                && r.path.len() <= 2048
                && r.path
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_-.~".contains(&b))
                && !r.path.split('/').any(|s| s == ".." || s == "."),
            "请求路径无效"
        );
        anyhow::ensure!(
            r.body.len() <= 8 * 1024 * 1024
                && r.headers.len() <= 8
                && r.query
                    .as_ref()
                    .is_none_or(|q| q.len() <= 8192 && !q.chars().any(char::is_control)),
            "请求超过限制"
        );
        let headers = r
            .headers
            .into_iter()
            .map(|h| {
                anyhow::ensure!(
                    ["content-type", "accept"].contains(&h.name.to_ascii_lowercase().as_str())
                        && h.value.len() <= 256
                        && !h.value.chars().any(char::is_control),
                    "前端不能注入身份或宿主请求头"
                );
                Ok(transport::Header {
                    name: h.name,
                    value: h.value,
                })
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self {
            method: r.method,
            path: r.path,
            query: r.query,
            headers,
            body: r.body,
        })
    }
}

#[derive(Serialize)]
pub(super) struct Response {
    pub status: u16,
    pub headers: Vec<Header>,
    /// 插件原始响应按 base64 回传，避免把字节摊成数字数组后再交给浏览器解析。
    #[serde(with = "base64_body")]
    pub body: Vec<u8>,
}
impl From<transport::Response> for Response {
    fn from(r: transport::Response) -> Self {
        Self {
            status: r.status,
            headers: r
                .headers
                .into_iter()
                .map(|h| Header {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
            body: r.body,
        }
    }
}

/// 插件正文在 JSON 信封里按 base64 传输。`Vec<u8>` 的默认表示是"一个字节一个数字"的数组，
/// 体积放大约 3.4 倍，宿主与浏览器都要为这座数字山付出序列化和解析成本。
mod base64_body {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize as _, Deserializer, Serializer, de::Error as _};

    pub(super) fn serialize<S>(body: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(body))
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        STANDARD
            .decode(encoded.as_bytes())
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_decodes_from_base64() {
        let request: Request = serde_json::from_value(serde_json::json!({
            "method": "POST",
            "path": "/tasks",
            "body": "AP+A",
        }))
        .expect("base64 正文必须能解析");
        assert_eq!(request.body, vec![0, 255, 128]);
    }

    #[test]
    fn missing_request_body_stays_empty() {
        let request: Request = serde_json::from_value(serde_json::json!({
            "method": "GET",
            "path": "/tasks",
        }))
        .expect("缺少正文必须按空正文处理");
        assert!(request.body.is_empty());
    }

    #[test]
    fn invalid_base64_body_is_rejected() {
        let error = serde_json::from_value::<Request>(serde_json::json!({
            "method": "POST",
            "path": "/tasks",
            "body": "not base64!",
        }))
        .expect_err("非法 base64 必须失败");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn legacy_number_array_body_fails_loudly() {
        // 旧宿主脚本发数字数组时必须显式失败，不能静默变成空正文。
        let error = serde_json::from_value::<Request>(serde_json::json!({
            "method": "POST",
            "path": "/tasks",
            "body": [0, 255, 128],
        }))
        .expect_err("旧数字数组协议必须被拒绝");
        assert!(error.to_string().contains("string"), "{error}");
    }

    #[test]
    fn response_body_serializes_as_base64() {
        let response = Response {
            status: 200,
            headers: Vec::new(),
            body: vec![0, 255, 128],
        };
        let value = serde_json::to_value(&response).expect("序列化桥响应");
        assert_eq!(value["body"], serde_json::json!("AP+A"));
        assert!(value["body"].is_string());
    }

    #[test]
    fn decoded_body_still_obeys_the_transport_limit() {
        use base64::Engine as _;
        let oversized =
            base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 8 * 1024 * 1024 + 1]);
        let request: Request = serde_json::from_value(serde_json::json!({
            "method": "POST",
            "path": "/tasks",
            "body": oversized,
        }))
        .expect("base64 正文必须能解析");
        assert!(transport::Request::try_from(request).is_err());
    }
}
