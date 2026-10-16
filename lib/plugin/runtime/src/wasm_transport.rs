//! WIT 编解码只在 Wasm 边界发生，公共执行器契约保持与引擎无关。

use az_plugin_contract::transport;

use crate::bindings::aio::plugin::transport as wit;

impl From<transport::Request> for wit::Request {
    fn from(request: transport::Request) -> Self {
        Self {
            method: request.method,
            path: request.path,
            query: request.query,
            headers: request.headers.into_iter().map(Into::into).collect(),
            body: request.body,
        }
    }
}

impl From<wit::Request> for transport::Request {
    fn from(request: wit::Request) -> Self {
        Self {
            method: request.method,
            path: request.path,
            query: request.query,
            headers: request.headers.into_iter().map(Into::into).collect(),
            body: request.body,
        }
    }
}

impl From<wit::Response> for transport::Response {
    fn from(response: wit::Response) -> Self {
        Self {
            status: response.status,
            headers: response.headers.into_iter().map(Into::into).collect(),
            body: response.body,
        }
    }
}

impl From<transport::Response> for wit::Response {
    fn from(response: transport::Response) -> Self {
        Self {
            status: response.status,
            headers: response.headers.into_iter().map(Into::into).collect(),
            body: response.body,
        }
    }
}

impl From<transport::Header> for wit::Header {
    fn from(header: transport::Header) -> Self {
        Self {
            name: header.name,
            value: header.value,
        }
    }
}

impl From<wit::Header> for transport::Header {
    fn from(header: wit::Header) -> Self {
        Self {
            name: header.name,
            value: header.value,
        }
    }
}
