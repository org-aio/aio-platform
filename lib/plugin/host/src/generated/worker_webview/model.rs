use serde::{Deserialize, Serialize};

/// 宿主从前端挂载推导身份，浏览器不能提交其他账号或插件来源。
#[derive(Clone)]
pub struct ViewOwner {
    pub runtime: WebviewRuntime,
    pub tenant: String,
    pub user: String,
    pub session: String,
    pub source: String,
    pub revision: String,
    pub mount: String,
}

#[cfg(feature = "server")]
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ViewRequest {
    List,
    Open {
        device: String,
        route: Option<String>,
    },
    Close {
        id: String,
    },
}

#[cfg(feature = "server")]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Access {
    pub enabled: bool,
    #[serde(default)]
    pub runtime: WebviewRuntime,
    #[serde(default)]
    pub headless: bool,
}

/// 网页只接收短期视图地址，不接收设备凭据或本机 CDP 地址。
#[cfg(feature = "server")]
#[derive(Serialize)]
pub(crate) struct OpenView {
    pub id: String,
    pub src: String,
}

/// 运行时由插件能力推导，浏览器不能跨插件切换设备授权。
#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebviewRuntime {
    #[default]
    Codex,
    Dsh,
}

impl WebviewRuntime {
    pub fn capability(self) -> &'static str {
        match self {
            Self::Codex => "codex.web",
            Self::Dsh => "dsh.web",
        }
    }
}
