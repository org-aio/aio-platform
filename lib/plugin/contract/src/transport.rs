//! 执行器与宿主之间的请求契约；不依赖 Wasm 引擎或任何传输协议。

/// 请求或响应中的单个头字段；身份和能力仍由宿主上下文提供。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// 宿主交给执行器的已授权请求，正文允许任意字节。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}

/// 执行器返回的响应；具体协议入口负责验证状态码和输出配额。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}
