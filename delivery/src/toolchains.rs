use anyhow::{Result, ensure};
use az_plugin_delivery::BuildEnvironment;

pub fn settings(
    environment: BuildEnvironment,
    proxy: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let values = match environment {
        BuildEnvironment::Rust => vec![
            ("NO_DOWNLOADS", "1"),
            ("DX_TELEMETRY_ENABLED", "false"),
            ("CARGO_HOME", "/cache/cargo"),
            ("CARGO_NET_GIT_FETCH_WITH_CLI", "false"),
            ("CARGO_UNSTABLE_GIT", "shallow-deps"),
            ("CARGO_HTTP_TIMEOUT", "30"),
            ("CARGO_HTTP_MULTIPLEXING", "false"),
            ("CARGO_NET_RETRY", "3"),
        ],
        BuildEnvironment::Kotlin => vec![
            ("KOTLIN_CLI_NO_WELCOME_BANNER", "1"),
            ("KOTLIN_CLI_JAVA_HOME", "/opt/java/openjdk"),
            (
                "JAVA_TOOL_OPTIONS",
                "-Duser.home=/cache -XX:ActiveProcessorCount=4",
            ),
        ],
        BuildEnvironment::TypeScript => vec![],
        BuildEnvironment::Fullstack => {
            let mut values = settings(BuildEnvironment::Rust, proxy)?;
            values.extend(settings(BuildEnvironment::Kotlin, proxy)?);
            return Ok(values);
        }
    };
    let mut values: Vec<(String, String)> = values
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    if let Some(proxy) = proxy {
        let url = url::Url::parse(proxy)?;
        ensure!(
            url.scheme() == "http" && url.username().is_empty() && url.password().is_none(),
            "构建代理必须是无凭据 HTTP 地址"
        );
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("代理缺少主机"))?;
        let port = url.port_or_known_default().unwrap_or(80);
        for (key, value) in &mut values {
            if key == "JAVA_TOOL_OPTIONS" {
                value.push_str(&format!(" -Dhttp.proxyHost={host} -Dhttp.proxyPort={port} -Dhttps.proxyHost={host} -Dhttps.proxyPort={port} -Dhttp.nonProxyHosts=localhost|127.*"));
            }
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_toolchains_share_the_configured_egress_without_credentials() -> Result<()> {
        let values = settings(BuildEnvironment::Fullstack, Some("http://172.17.0.1:17892"))?;
        assert!(
            values
                .iter()
                .any(|(key, value)| key == "CARGO_HOME" && value == "/cache/cargo")
        );
        let java = &values
            .iter()
            .find(|(key, _)| key == "JAVA_TOOL_OPTIONS")
            .unwrap()
            .1;
        assert!(java.contains("-Dhttps.proxyHost=172.17.0.1 -Dhttps.proxyPort=17892"));
        assert!(java.contains("-Duser.home=/cache"));
        assert!(settings(BuildEnvironment::Kotlin, Some("http://user:password@proxy")).is_err());
        Ok(())
    }
}
