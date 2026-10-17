use std::time::Duration;

use anyhow::{Context, Result, ensure};
use az_plugin_contract::storage::{MAX_OBJECT_BYTES, MAX_OBJECT_KEY_BYTES, OBJECT_TIMEOUT_SECONDS};
use reqwest::{Client, Method, RequestBuilder, StatusCode, Url};

use super::Host;

/// 通过宿主 Unix broker 访问当前插件、当前租户的对象；不持有宿主磁盘路径。
#[derive(Clone)]
pub struct ObjectStorage {
    client: Client,
}

impl Host {
    /// 创建有界的对象客户端。插件须声明 storage 能力，权限仍由宿主实时检查。
    pub fn storage(&self) -> Result<ObjectStorage> {
        ensure!(self.configuration.storage, "宿主未授予对象存储");
        let mut headers = reqwest::header::HeaderMap::new();
        let mut token = reqwest::header::HeaderValue::from_str(&self.configuration.ingress_token)?;
        token.set_sensitive(true);
        headers.insert("x-aio-token", token);
        let client = Client::builder()
            .unix_socket(self.configuration.broker_socket.as_str())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(OBJECT_TIMEOUT_SECONDS + 2))
            .build()?;
        Ok(ObjectStorage { client })
    }
}

impl ObjectStorage {
    fn request(&self, method: Method, key: &str) -> Result<RequestBuilder> {
        ensure!(
            !key.is_empty() && key.len() <= MAX_OBJECT_KEY_BYTES,
            "对象键长度无效"
        );
        let mut url = Url::parse("http://localhost/storage")?;
        url.query_pairs_mut().append_pair("key", key);
        Ok(self.client.request(method, url))
    }

    /// 读取有界二进制对象；不存在返回 None，撤权和后端故障返回错误。
    pub async fn read(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let mut response = self
            .request(Method::GET, key)?
            .send()
            .await
            .context("读取宿主对象失败")?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            response.status() == StatusCode::OK,
            "读取宿主对象失败: HTTP {}",
            response.status()
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size <= MAX_OBJECT_BYTES as u64),
            "对象超过 16 MiB 配额"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                chunk.len() <= MAX_OBJECT_BYTES - bytes.len(),
                "对象超过 16 MiB 配额"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }

    /// 写入完整对象，键由宿主哈希隔离，载荷不经过 JSON/base64 扩张。
    pub async fn write(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        ensure!(bytes.len() <= MAX_OBJECT_BYTES, "对象超过 16 MiB 配额");
        let response = self
            .request(Method::PUT, key)?
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .context("写入宿主对象失败")?;
        ensure!(
            response.status() == StatusCode::NO_CONTENT,
            "写入宿主对象失败: HTTP {}",
            response.status()
        );
        Ok(())
    }

    /// 幂等删除对象；网络中断后的重试仍由业务协调。
    pub async fn remove(&self, key: &str) -> Result<()> {
        let response = self
            .request(Method::DELETE, key)?
            .send()
            .await
            .context("删除宿主对象失败")?;
        ensure!(
            response.status() == StatusCode::NO_CONTENT,
            "删除宿主对象失败: HTTP {}",
            response.status()
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
