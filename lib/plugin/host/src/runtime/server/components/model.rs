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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
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
