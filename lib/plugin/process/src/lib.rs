//! 原生进程插件的宿主配置和可信入口；不依赖宿主、数据库或具体业务。

mod ingress;
#[cfg(unix)]
mod storage;
#[cfg(unix)]
pub use storage::ObjectStorage;

use std::{io::Read, path::PathBuf};

use anyhow::{Context, Result, ensure};
use axum::Router;
use az_plugin_contract::{ABI_VERSION, process::Configuration};

pub use ingress::Caller;

/// 由 Sidecar 运行时发放的配置；业务从配置获取受限资源，通过路由入口接收可信调用。
pub struct Host {
    pub configuration: Configuration,
    socket: PathBuf,
}

impl Host {
    /// 读取宿主发放的文件与 Unix socket；缺失配置时拒绝退回非认证模式。
    pub fn from_env() -> Result<Self> {
        let path = std::env::var_os("AIO_PLUGIN_CONFIG").context("缺少 AIO_PLUGIN_CONFIG")?;
        let socket = std::env::var_os("AIO_PLUGIN_SOCKET").context("缺少 AIO_PLUGIN_SOCKET")?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(65537)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes, socket.into())
    }

    /// 解析有界配置。仅接受当前 ABI、明确租户和绝对路径。
    pub fn from_bytes(bytes: &[u8], socket: PathBuf) -> Result<Self> {
        ensure!(bytes.len() <= 65536, "宿主配置超过配额");
        let configuration: Configuration = serde_json::from_slice(bytes)?;
        ensure!(
            configuration.abi_version == ABI_VERSION
                && ingress::valid_identity(&configuration.tenant_id)
                && (32..=256).contains(&configuration.ingress_token.len())
                && configuration
                    .ingress_token
                    .bytes()
                    .all(|b| b.is_ascii_graphic())
                && PathBuf::from(&configuration.broker_socket).is_absolute()
                && socket.is_absolute(),
            "宿主配置绑定无效"
        );
        Ok(Self {
            configuration,
            socket,
        })
    }

    /// 在全部业务路由前验证宿主令牌和租户；健康及描述路由由调用方在外部合并。
    pub fn protect(&self, router: Router) -> Router {
        ingress::protect(router, &self.configuration)
    }

    /// 只监听宿主分配的 Unix socket；收到终止信号后先排空请求，再由业务关闭资源。
    #[cfg(unix)]
    pub async fn serve(self, router: Router) -> Result<()> {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let listener =
            tokio::net::UnixListener::bind(&self.socket).context("绑定插件 Unix socket 失败")?;
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    _ = terminate.recv() => {},
                    _ = interrupt.recv() => {},
                }
            })
            .await
            .context("插件 HTTP 服务失败")
    }
}

#[cfg(test)]
mod tests;
