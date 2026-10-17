//! GitHub 只负责读取仓库与固定提交的清单，持久化与入队交给 DeliverySources。
use super::{DeliverySource, DeliverySources, DeliveryTarget};
use crate::configuration::DeliveryConfig;
use anyhow::{Context as _, Result, ensure};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Repository {
    clone_url: String,
    full_name: String,
    default_branch: String,
    archived: bool,
    fork: bool,
}

/// 每代发现插件拥有自己的客户端与分页缓存，不保存运行时或数据库引用。
#[derive(Clone)]
pub(crate) struct GithubDiscovery {
    client: Client,
    base: reqwest::Url,
    token: Option<String>,
    config: DeliveryConfig,
}
impl GithubDiscovery {
    pub(crate) fn new(base: &str, config: DeliveryConfig) -> Result<Self> {
        ensure!(
            !config.owner.is_empty()
                && config
                    .owner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "自动发现 owner 无效"
        );
        let base = reqwest::Url::parse(base)?;
        let loopback = base
            .host_str()
            .and_then(|h| h.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        ensure!(
            base.scheme() == "https" || (base.scheme() == "http" && loopback),
            "GitHub API 必须使用 HTTPS，回环测试地址除外"
        );
        ensure!(
            base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none()
                && base.path().ends_with('/'),
            "GitHub API 地址必须是无凭据且以 / 结尾的基础地址"
        );
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base,
            token: std::env::var("AIO_DELIVERY_GITHUB_TOKEN").ok(),
            config,
        })
    }
    fn request(&self, path: &str) -> Result<reqwest::RequestBuilder> {
        let request = self
            .client
            .get(self.base.join(path)?)
            .header("user-agent", "aio-delivery")
            .header("accept", "application/vnd.github+json");
        Ok(match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        })
    }
    async fn response(
        &self,
        request: reqwest::RequestBuilder,
        cancel: &CancellationToken,
        limit: usize,
    ) -> Result<(StatusCode, Option<String>, Vec<u8>)> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => anyhow::bail!("发现任务已取消"),
            result = async {
                let mut response = request.send().await?;
                let status = response.status();
                let etag = response.headers().get("etag").and_then(|h| h.to_str().ok()).map(str::to_owned);
                if status == StatusCode::NOT_FOUND || status == StatusCode::NOT_MODIFIED { return Ok((status, etag, Vec::new())); }
                ensure!(status.is_success(), "GitHub API 返回 {status}");
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    ensure!(chunk.len() <= limit.saturating_sub(body.len()), "GitHub API 响应过大");
                    body.extend_from_slice(&chunk);
                }
                Ok((status, etag, body))
            } => result
        }
    }
    async fn inspect(
        &self,
        sources: &dyn DeliverySources,
        repo: &str,
        git: &str,
        branch: &str,
        current: Option<DeliverySource>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Commit {
            sha: String,
        }
        // 网络访问前取得代数；陈旧响应不能覆盖后来已提交的观察或撤销。
        let expected = current.as_ref().map(|s| s.generation);
        let request = self
            .request(&format!("repos/{repo}/commits"))?
            .query(&[("sha", branch), ("per_page", "1")]);
        let (status, _, bytes) = self.response(request, cancel, 1024 * 1024).await?;
        ensure!(status.is_success(), "不能读取仓库 {repo} 的默认分支");
        let commits: Vec<Commit> = serde_json::from_slice(&bytes)?;
        let sha = commits.into_iter().next().context("默认分支为空")?.sha;
        ensure!(
            sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "无效提交 SHA"
        );
        if current
            .as_ref()
            .is_some_and(|s| s.enabled && s.source_revision == sha && s.branch == branch)
        {
            return Ok(());
        }
        let request = self
            .request(&format!("repos/{repo}/contents/aio-delivery.toml"))?
            .header("accept", "application/vnd.github.raw+json")
            .query(&[("ref", &sha)]);
        let (status, _, bytes) = self.response(request, cancel, 16384).await?;
        let target = if status == StatusCode::NOT_FOUND {
            // 未登记且无清单的普通仓库无需写入；已有来源需要真正撤销。
            if current.is_none() {
                return Ok(());
            }
            None
        } else {
            let manifest = az_plugin_delivery::parse(std::str::from_utf8(&bytes)?)?;
            Some(DeliveryTarget {
                branch: branch.into(),
                source_revision: sha,
                recipe: manifest.build,
            })
        };
        if cancel.is_cancelled() {
            return Ok(());
        }
        ensure!(
            sources.update(git, expected, target).await?,
            "交付来源在检查期间已变化，需要重新读取"
        );
        Ok(())
    }
    async fn scan(
        &self,
        sources: Arc<dyn DeliverySources>,
        etags: &mut HashMap<usize, (String, bool)>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        for page in 1..=100 {
            if cancel.is_cancelled() {
                break;
            }
            let mut request = self
                .request(&format!("users/{}/repos", self.config.owner))?
                .query(&[
                    ("per_page", "100".to_string()),
                    ("page", page.to_string()),
                    ("sort", "updated".to_string()),
                ]);
            if let Some((etag, _)) = etags.get(&page) {
                request = request.header("if-none-match", etag);
            }
            let (status, etag, bytes) = self.response(request, cancel, 4 * 1024 * 1024).await?;
            if status == StatusCode::NOT_MODIFIED {
                if etags.get(&page).is_some_and(|(_, last)| *last) {
                    break;
                }
                continue;
            }
            ensure!(status.is_success(), "无法列举 GitHub 仓库");
            let repositories: Vec<Repository> = serde_json::from_slice(&bytes)?;
            let last = repositories.len() < 100;
            let mut tasks = tokio::task::JoinSet::new();
            let mut failed = false;
            for repository in repositories {
                if cancel.is_cancelled() {
                    break;
                }
                let discovery = self.clone();
                let sources = sources.clone();
                let cancel = cancel.clone();
                tasks.spawn(async move {
                    let result = discovery.repository(sources, &repository, &cancel).await;
                    if let Err(error) = &result {
                        eprintln!("检查 {} 发布来源失败: {error:#}", repository.full_name);
                    }
                    result
                });
                if tasks.len() >= 8 {
                    failed |= !matches!(tasks.join_next().await, Some(Ok(Ok(()))));
                }
            }
            while let Some(result) = tasks.join_next().await {
                failed |= !matches!(result, Ok(Ok(())));
            }
            if failed {
                etags.remove(&page);
            } else if let Some(etag) = etag {
                etags.insert(page, (etag, last));
            }
            if last {
                etags.retain(|p, _| *p <= page);
                break;
            }
        }
        Ok(())
    }
    async fn repository(
        &self,
        sources: Arc<dyn DeliverySources>,
        repository: &Repository,
        cancel: &CancellationToken,
    ) -> Result<()> {
        ensure!(
            self.repository_name(&repository.clone_url) == Some(repository.full_name.as_str()),
            "GitHub 返回不属于当前 API 的仓库地址"
        );
        if repository.archived || repository.fork {
            if let Some(source) = sources.get(&repository.clone_url).await?
                && source.enabled
                && !cancel.is_cancelled()
            {
                ensure!(
                    sources
                        .update(&source.git, Some(source.generation), None)
                        .await?,
                    "撤销期间来源已变化"
                );
            }
            return Ok(());
        }
        let current = sources.get(&repository.clone_url).await?;
        if current.is_none() {
            // 普通或空仓库没有交付标记时不查询提交；真正入队仍按固定 SHA 重读清单。
            let request = self
                .request(&format!(
                    "repos/{}/contents/aio-delivery.toml",
                    repository.full_name
                ))?
                .header("accept", "application/vnd.github.raw+json")
                .query(&[("ref", &repository.default_branch)]);
            let (status, _, bytes) = self.response(request, cancel, 16384).await?;
            if status == StatusCode::NOT_FOUND {
                return Ok(());
            }
            az_plugin_delivery::parse(std::str::from_utf8(&bytes)?)?;
        }
        self.inspect(
            sources.as_ref(),
            &repository.full_name,
            &repository.clone_url,
            &repository.default_branch,
            current,
            cancel,
        )
        .await
    }
    fn repository_name<'a>(&self, git: &'a str) -> Option<&'a str> {
        // 只轮询本适配器负责的仓库；其他 Git 服务的来源由其他发现插件维护。
        let host = if self.base.host_str() == Some("api.github.com") {
            "github.com"
        } else {
            self.base.host_str()?
        };
        let repo = git
            .strip_prefix(&format!("https://{host}/"))?
            .strip_suffix(".git")?;
        let mut parts = repo.split('/');
        let owner = parts.next()?;
        let name = parts.next()?;
        if !owner.eq_ignore_ascii_case(&self.config.owner)
            || matches!(name, "" | "." | "..")
            || parts.next().is_some()
            || !repo
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
        {
            return None;
        }
        Some(repo)
    }
    async fn poll(
        &self,
        sources: Arc<dyn DeliverySources>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut tasks = tokio::task::JoinSet::new();
        let mut failed = false;
        for source in sources.enabled().await? {
            if cancel.is_cancelled() {
                break;
            }
            let Some(repo) = self.repository_name(&source.git) else {
                continue;
            };
            let repo = repo.to_owned();
            let discovery = self.clone();
            let sources = sources.clone();
            let cancel = cancel.clone();
            tasks.spawn(async move {
                let git = source.git.clone();
                let branch = source.branch.clone();
                if let Err(error) = discovery
                    .inspect(
                        sources.as_ref(),
                        &repo,
                        &git,
                        &branch,
                        Some(source),
                        &cancel,
                    )
                    .await
                {
                    eprintln!("检查 {repo} 失败: {error:#}");
                }
            });
            if tasks.len() >= 8 {
                failed |= !matches!(tasks.join_next().await, Some(Ok(())));
            }
        }
        while let Some(result) = tasks.join_next().await {
            failed |= result.is_err();
        }
        ensure!(!failed, "来源轮询任务异常终止");
        Ok(())
    }
    pub(crate) async fn run(
        self: Arc<Self>,
        sources: Arc<dyn DeliverySources>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let mut discover = tokio::time::interval(Duration::from_secs(
            self.config.discovery_interval_seconds.max(1),
        ));
        let mut revisions = tokio::time::interval(Duration::from_secs(
            self.config.revision_interval_seconds.max(1),
        ));
        discover.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        revisions.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut etags = HashMap::new();
        loop {
            // 同一发现器串行执行两类扫描；跨进程或其他插件的竞争由来源代数约束。
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = discover.tick() => self.scan(sources.clone(), &mut etags, &cancel).await,
                _ = revisions.tick() => self.poll(sources.clone(), &cancel).await,
            };
            if let Err(error) = result
                && !cancel.is_cancelled()
            {
                eprintln!("交付发现失败: {error:#}");
            }
        }
        Ok(())
    }
}
