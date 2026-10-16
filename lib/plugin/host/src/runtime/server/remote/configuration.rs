use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::super::plugins::RuntimeCapability;

pub(super) const MAX_WIRE_BYTES: usize = 1024 * 1024;
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 只有部署方可修改此文件；远端地址与凭据不来自脚本或普通租户请求。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub schema_version: u32,
    pub imports: Vec<Import>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Import {
    pub tenant_id: String,
    pub capability: RuntimeCapability,
    pub endpoint: String,
    pub credential_env: Option<String>,
    pub transport: Transport,
}

#[derive(Deserialize)]
#[serde(tag = "protocol", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Transport {
    Mcp {
        tool: String,
        output: McpOutput,
    },
    Grpc {
        descriptor_set: PathBuf,
        service: String,
        method: String,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum McpOutput {
    Structured,
    Envelope,
}

/// 配置和描述文件先读为同一候选的内存内容，连接校验期间不再读取变化中的文件。
pub(super) struct Loaded {
    pub revision: String,
    pub imports: Vec<(Import, Option<Vec<u8>>)>,
}

impl Loaded {
    pub async fn read(path: &Path) -> Result<Self> {
        let bytes = read(path).await?;
        let config: Configuration = serde_json::from_slice(&bytes)?;
        ensure!(config.schema_version == 1, "不支持的远端能力配置版本");
        ensure!(config.imports.len() <= 64, "远端能力绑定数不能超过 64");
        let mut digest = Sha256::new();
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
        let mut imports = Vec::new();
        let mut total = bytes.len();
        for import in config.imports {
            let descriptor = if let Transport::Grpc { descriptor_set, .. } = &import.transport {
                let descriptor = path.parent().unwrap_or(Path::new(".")).join(descriptor_set);
                let bytes = read(&descriptor).await?;
                total += bytes.len();
                ensure!(
                    total <= 8 * MAX_WIRE_BYTES,
                    "远端能力候选文件总量超过 8 MiB"
                );
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(&bytes);
                Some(bytes)
            } else {
                None
            };
            imports.push((import, descriptor));
        }
        Ok(Self {
            revision: format!("{:x}", digest.finalize()),
            imports,
        })
    }
}

pub(super) async fn read(path: &Path) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path)
        .await
        .context("打开远端能力文件失败")?;
    let mut bytes = Vec::new();
    file.take((MAX_WIRE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= MAX_WIRE_BYTES, "远端能力文件超过 1 MiB");
    Ok(bytes)
}

pub(super) fn endpoint(value: &str, grpc: bool) -> Result<()> {
    let url = az_plugin_contract::process::third_party_http_endpoint(value)
        .map_err(|error| anyhow::anyhow!(error))?;
    let host = url.host_str().context("远端能力缺少地址")?;
    let local = host
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| match ip {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
        });
    ensure!(
        url.scheme() == "https" || local,
        "远端能力必须使用 HTTPS 或固定本机/私网 IP"
    );
    ensure!(
        !grpc || url.path() == "/",
        "gRPC 地址不能包含方法或路径前缀"
    );
    Ok(())
}

pub(super) fn credential(name: Option<&str>) -> Result<Option<axum::http::HeaderValue>> {
    let Some(name) = name else {
        return Ok(None);
    };
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'),
        "凭据环境变量名无效"
    );
    let value =
        std::env::var(name).map_err(|_| anyhow::anyhow!("远端能力凭据未配置或编码无效: {name}"))?;
    ensure!(!value.trim().is_empty(), "远端能力凭据为空");
    let mut header = axum::http::HeaderValue::from_str(&format!("Bearer {value}"))
        .context("远端能力凭据不是有效认证头")?;
    header.set_sensitive(true);
    Ok(Some(header))
}
