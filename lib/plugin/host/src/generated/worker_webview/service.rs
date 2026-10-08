use super::model::ViewOwner;
use crate::{
    generated::worker::model::{DeviceIdentity, Worker},
    runtime::server::http_error::RuntimeError,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// 设备长连接与网页连接的临时传输由同一服务管理；帧不落库。
#[async_trait::async_trait]
pub(crate) trait WorkerWebviewService: Send + Sync {
    async fn access(&self, device: &DeviceIdentity, enabled: bool) -> Result<(), RuntimeError>;
    async fn devices(&self, owner: &ViewOwner) -> Result<Vec<Worker>, RuntimeError>;
    async fn create(
        &self,
        owner: &ViewOwner,
        device: &str,
        route: &str,
    ) -> Result<String, RuntimeError>;
    async fn authorize(&self, owner: &ViewOwner, id: &str) -> Result<String, RuntimeError>;
    async fn register(
        &self,
        device: &DeviceIdentity,
    ) -> Result<(String, mpsc::Receiver<Value>), RuntimeError>;
    async fn expire(&self, device: &str) -> Result<(), RuntimeError>;
    async fn unregister(&self, device: &str, generation: &str);
    async fn attach(
        &self,
        owner: &ViewOwner,
        id: &str,
    ) -> Result<mpsc::Receiver<Value>, RuntimeError>;
    async fn frame(&self, owner: &ViewOwner, id: &str, frame: Value) -> Result<(), RuntimeError>;
    async fn asset(&self, owner: &ViewOwner, id: &str, path: &str) -> Result<Value, RuntimeError>;
    async fn receive(
        &self,
        device: &str,
        generation: &str,
        frame: Value,
    ) -> Result<(), RuntimeError>;
    async fn close(&self, owner: &ViewOwner, id: &str) -> Result<(), RuntimeError>;
}

pub(super) struct Peer {
    pub generation: String,
    pub sender: mpsc::Sender<Value>,
}

pub(super) struct ViewChannel {
    pub worker: String,
    pub sender: mpsc::Sender<Value>,
    pub receiver: Option<mpsc::Receiver<Value>>,
    pub assets: std::collections::HashMap<String, oneshot::Sender<Value>>,
}
