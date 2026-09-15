/// 校验已声明并获宿主批准的模型基址；HTTP 仅接受固定的私网 IP。
/// 宿主仍须逐项核对允许列表，不能据此自动授权地址。
pub fn model_endpoint(value: &str) -> Result<url::Url, &'static str> {
    let url = url::Url::parse(value).map_err(|_| "模型地址无效")?;
    let private = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_private(),
        Some(url::Host::Ipv6(ip)) => ip.is_unique_local(),
        _ => false,
    };
    if value.len() > 2048
        || value.ends_with('/')
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (url.scheme() == "http" && private))
    {
        return Err("模型地址必须为无凭据的完整 HTTPS 基址或已授权私网 IP 的 HTTP 基址");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::model_endpoint;

    #[test]
    fn permits_https_and_fixed_private_model_addresses() {
        for value in [
            "https://api.openai.com/v1",
            "http://192.168.31.252:18080/v1",
            "http://10.10.0.4:8080/v1",
            "http://172.16.0.3:8080/v1",
            "http://[fd00::5]:8080/v1",
        ] {
            assert!(model_endpoint(value).is_ok(), "{value}");
        }
    }

    #[test]
    fn rejects_public_http_metadata_addresses_and_credential_urls() {
        for value in [
            "http://example.com/v1",
            "http://localhost:11434/v1",
            "http://127.0.0.1/v1",
            "http://169.254.169.254/latest",
            "http://100.100.100.200/latest",
            "http://172.32.0.1/v1",
            "http://[fe80::1]/v1",
            "http://[::1]/v1",
            "https://user:secret@host/v1",
            "https://host/v1?key=secret",
            "https://host/v1#fragment",
            "https://host/v1/",
            "file:///etc/passwd",
        ] {
            assert!(model_endpoint(value).is_err(), "{value}");
        }
    }
}
