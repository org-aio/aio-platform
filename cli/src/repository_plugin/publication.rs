use std::{env, io::Read, net::IpAddr, path::PathBuf, thread, time::Duration};

use anyhow::{Context as _, Result, bail, ensure};
use az_plugin_package::{PACKAGE_CONTENT_TYPE, PluginPackage};
use reqwest::{
    Url,
    blocking::{Client, Response},
    header,
};
use serde::Deserialize;

use super::packaging::{prepare_package, read_package};

const PUBLISH_URL_ENV: &str = "AIO_PLUGIN_PUBLISH_URL";
const PUBLISH_TOKEN_ENV: &str = "AIO_PLUGIN_PUBLISH_TOKEN";
pub(crate) const DEFAULT_PUBLISH_URL: &str = "https://aio.addzero.site/api/runtime/plugins/publish";
const POLL_ATTEMPTS: usize = 90;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: u64 = 128 * 1024;

#[derive(Debug)]
pub struct PublicationOptions {
    pub root: PathBuf,
    pub git: Option<String>,
    pub version: Option<String>,
}

pub fn publish(options: PublicationOptions) -> Result<()> {
    let package = if options.root.is_file() {
        read_package(
            &options.root,
            options.git.as_deref(),
            options.version.as_deref(),
        )?
    } else {
        prepare_package(&options.root, options.git, options.version)?
    };
    let endpoint = env::var(PUBLISH_URL_ENV).unwrap_or_else(|_| DEFAULT_PUBLISH_URL.to_owned());
    let token = env::var(PUBLISH_TOKEN_ENV)
        .with_context(|| format!("缺少 {PUBLISH_TOKEN_ENV}，请先在插件市场创建发布凭证"))?;
    ensure!(!token.trim().is_empty(), "{PUBLISH_TOKEN_ENV} 不能为空");
    let active = publish_to(&package, &endpoint, &token)?;
    println!(
        "插件发布已激活: version={} revision={} pages={} detail={}",
        package.version, active.revision, active.page_count, active.detail
    );
    Ok(())
}

#[derive(Deserialize)]
struct RuntimeResponse<T> {
    data: T,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PublishState {
    Queued,
    Running,
    Active,
    Failed,
}

#[derive(Deserialize)]
struct PublishedPlugin {
    job_id: String,
    revision: String,
    page_count: usize,
    state: PublishState,
    detail: String,
}

fn publish_to(package: &PluginPackage, endpoint: &str, token: &str) -> Result<PublishedPlugin> {
    let endpoint = publish_url(endpoint)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("创建插件发布客户端失败")?;
    let response = client
        .post(endpoint.clone())
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
        .body(package.encode()?)
        .send()
        .context("请求插件发布接口失败")?;
    let mut status = decode_response(response)?;
    let job_id = status.job_id.clone();
    let job_url = publish_job_url(endpoint, &job_id)?;
    for attempt in 0..=POLL_ATTEMPTS {
        ensure!(
            status.revision == package.rev,
            "发布接口返回了不一致的内容版本"
        );
        ensure!(status.job_id == job_id, "发布接口返回了不一致的任务 ID");
        match status.state {
            PublishState::Active => return Ok(status),
            PublishState::Failed => bail!("插件发布失败: {}", status.detail),
            PublishState::Queued | PublishState::Running => {}
        }
        ensure!(
            attempt < POLL_ATTEMPTS,
            "等待插件发布激活超时，任务 ID: {job_id}"
        );
        thread::sleep(POLL_INTERVAL);
        let response = client
            .get(job_url.clone())
            .bearer_auth(token)
            .send()
            .context("查询插件发布任务失败")?;
        status = decode_response(response)?;
    }
    unreachable!()
}

fn decode_response(response: Response) -> Result<PublishedPlugin> {
    let status = response.status();
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .context("读取插件发布响应失败")?;
    ensure!(body.len() as u64 <= MAX_RESPONSE_BYTES, "插件发布响应过大");
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&body);
        bail!(
            "插件发布接口返回 {status}: {}",
            detail.chars().take(4096).collect::<String>()
        );
    }
    serde_json::from_slice::<RuntimeResponse<PublishedPlugin>>(&body)
        .context("解析插件发布响应失败")
        .map(|response| response.data)
}

fn publish_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("AIO_PLUGIN_PUBLISH_URL 不是有效 URL")?;
    let loopback_http = url.scheme() == "http" && url_is_loopback(&url);
    ensure!(
        (url.scheme() == "https" || loopback_http)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path().ends_with("/plugins/publish"),
        "AIO_PLUGIN_PUBLISH_URL 必须是 HTTPS 发布接口；仅本机开发允许 HTTP"
    );
    Ok(url)
}

fn url_is_loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

fn publish_job_url(mut endpoint: Url, job_id: &str) -> Result<Url> {
    ensure!(
        !job_id.is_empty()
            && job_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "发布任务 ID 无效"
    );
    let base = endpoint
        .path()
        .strip_suffix("/plugins/publish")
        .context("发布接口路径无效")?;
    endpoint.set_path(&format!("{base}/publish-jobs/{job_id}"));
    Ok(endpoint)
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
