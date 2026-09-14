use std::{
    env,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    str::FromStr as _,
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};

pub(super) struct RemoteResolution {
    pub host: String,
    pub socket: SocketAddr,
}

impl RemoteResolution {
    pub fn git_configuration(&self) -> String {
        let address = match self.socket.ip() {
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("[{address}]"),
        };
        format!(
            "http.curloptResolve={}:{}:{address}",
            self.host,
            self.socket.port()
        )
    }
}

pub(super) fn validate_git(git: &str) -> Result<()> {
    ensure!(
        !git.contains(char::is_whitespace),
        "插件 Git 地址不能包含空白字符"
    );
    let source = reqwest::Url::parse(git).context("插件 Git 地址无效")?;
    ensure!(
        source.scheme() == "https"
            && source.host_str().is_some()
            && source.path().ends_with(".git")
            && source.port_or_known_default() == Some(443),
        "插件来源必须是 HTTPS Git 仓库"
    );
    ensure!(
        source.username().is_empty() && source.password().is_none(),
        "插件 Git 地址不能包含凭据"
    );
    ensure!(
        source.query().is_none() && source.fragment().is_none(),
        "插件 Git 地址不能包含查询参数或片段"
    );
    validate_remote_literal(&source)
}

fn validate_remote_literal(source: &reqwest::Url) -> Result<()> {
    let host = source.host_str().context("远程地址缺少主机")?;
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    ensure!(
        normalized != "localhost"
            && !normalized.ends_with(".localhost")
            && !normalized.ends_with(".local")
            && !normalized.ends_with(".internal")
            && normalized != "home.arpa"
            && !normalized.ends_with(".home.arpa"),
        "远程地址不能指向本机或内部域名"
    );
    if let Ok(address) = IpAddr::from_str(&normalized) {
        ensure!(is_public_ip(address), "远程地址不能指向非公网 IP");
    }
    Ok(())
}

pub(super) async fn validate_public_remote(source: &str) -> Result<RemoteResolution> {
    let source = reqwest::Url::parse(source).context("远程地址无效")?;
    validate_remote_literal(&source)?;
    let host = source.host_str().context("远程地址缺少主机")?;
    let port = source.port_or_known_default().context("远程地址缺少端口")?;
    let addresses = tokio::time::timeout(git_timeout(), tokio::net::lookup_host((host, port)))
        .await
        .context("解析远程地址超时")??
        .collect::<Vec<_>>();
    ensure!(!addresses.is_empty(), "远程地址没有可用 DNS 记录");
    ensure!(
        addresses.iter().all(|address| is_public_ip(address.ip())),
        "远程地址解析到了非公网 IP"
    );
    let socket = addresses
        .iter()
        .find(|address| address.is_ipv4())
        .copied()
        .unwrap_or(addresses[0]);
    Ok(RemoteResolution {
        host: host.to_owned(),
        socket,
    })
}

pub(super) fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    !address.is_private()
        && !address.is_loopback()
        && !address.is_link_local()
        && !address.is_broadcast()
        && !address.is_documentation()
        && !address.is_unspecified()
        && !address.is_multicast()
        && first != 0
        && !(first == 100 && (64..=127).contains(&second))
        && !(first == 192 && second == 0)
        && !(first == 198 && (18..=19).contains(&second))
        && first < 240
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(address) = address.to_ipv4_mapped() {
        return is_public_ipv4(address);
    }
    let segments = address.segments();
    (segments[0] & 0xe000) == 0x2000
        && !address.is_loopback()
        && !address.is_unspecified()
        && !address.is_unique_local()
        && !address.is_unicast_link_local()
        && !address.is_multicast()
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
}

pub(super) fn git_timeout() -> Duration {
    env::var("AIO_GIT_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(30))
}
