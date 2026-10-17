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
    pub runtime: String,
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

/// 原生插件声明的设备通道契约；protocol 是跨进程协议值，不参与 Dill 类型身份。
/// 浏览器不能提交能力或放宽文档策略，宿主只从已装配提供者取得这些字段。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebviewRuntime {
    pub protocol: String,
    pub capability: String,
    pub headless_capability: Option<String>,
    pub document: Option<WebviewDocument>,
}

/// 可嵌入设备网页的脚本策略；没有文档策略的通道只供原生服务消费。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebviewDocument {
    pub allow_eval: bool,
}
