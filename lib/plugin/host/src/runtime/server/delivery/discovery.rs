use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use sqlx::Row;

use super::super::RuntimeState;

#[derive(Deserialize)]
struct Repository {
    clone_url: String,
    full_name: String,
    default_branch: String,
    archived: bool,
    fork: bool,
}

fn request(client: &Client, path: &str) -> reqwest::RequestBuilder {
    let request = client
        .get(format!("https://api.github.com/{path}"))
        .header("user-agent", "aio-delivery")
        .header("accept", "application/vnd.github+json");
    match std::env::var("AIO_DELIVERY_GITHUB_TOKEN") {
        Ok(token) => request.bearer_auth(token),
        Err(_) => request,
    }
}

async fn inspect(
    state: &RuntimeState,
    client: &Client,
    repo: &str,
    git: &str,
    branch: &str,
) -> Result<()> {
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let commits: Vec<Commit> = request(client, &format!("repos/{repo}/commits"))
        .query(&[("sha", branch), ("per_page", "1")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let sha = commits.into_iter().next().context("默认分支为空")?.sha;
    ensure!(
        sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
        "无效提交 SHA"
    );
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM delivery_sources WHERE git=$1 AND desired_sha=$2 AND branch=$3 AND enabled)",
    )
    .bind(git)
    .bind(&sha)
    .bind(branch)
    .fetch_one(&state.store.pool)
    .await?;
    if exists {
        return Ok(());
    }
    let response = request(client, &format!("repos/{repo}/contents/aio-delivery.toml"))
        .header("accept", "application/vnd.github.raw+json")
        .query(&[("ref", sha.as_str())])
        .send()
        .await?;
    if response.status() == StatusCode::NOT_FOUND {
        sqlx::query("UPDATE delivery_sources SET enabled=FALSE,updated_at=now() WHERE git=$1")
            .bind(git)
            .execute(&state.store.pool)
            .await?;
        return Ok(());
    }
    let manifest = az_plugin_delivery::parse(&response.error_for_status()?.text().await?)?;
    let mut tx = state.store.pool.begin().await?;
    sqlx::query("INSERT INTO delivery_sources(git,branch,desired_sha) VALUES($1,$2,$3) ON CONFLICT(git) DO UPDATE SET branch=EXCLUDED.branch,desired_sha=EXCLUDED.desired_sha,enabled=TRUE,updated_at=now()")
        .bind(git).bind(branch).bind(&sha).execute(&mut *tx).await?;
    sqlx::query("UPDATE delivery_jobs SET state='superseded',lease_until=NULL,updated_at=now() WHERE git=$1 AND source_revision<>$2 AND state IN ('queued','building','uploaded','publishing')").bind(git).bind(&sha).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO delivery_jobs(git,source_revision,recipe) VALUES($1,$2,$3) ON CONFLICT(git,source_revision) DO NOTHING")
        .bind(git).bind(&sha).bind(serde_json::to_value(manifest.build)?).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn scan(
    state: &RuntimeState,
    client: &Client,
    owner: &str,
    etags: &mut std::collections::HashMap<usize, (String, bool)>,
) -> Result<()> {
    for page in 1..=100 {
        let mut request = request(
            client,
            &format!("users/{owner}/repos?per_page=100&page={page}&sort=updated"),
        );
        if let Some((etag, _)) = etags.get(&page) {
            request = request.header("if-none-match", etag);
        }
        let response = request.send().await?;
        if response.status() == StatusCode::NOT_MODIFIED {
            if etags.get(&page).is_some_and(|(_, last)| *last) {
                break;
            }
            continue;
        }
        let response = response.error_for_status()?;
        let etag = response
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let repositories: Vec<Repository> = response.json().await?;
        let last = repositories.len() < 100;
        let mut tasks = tokio::task::JoinSet::new();
        let mut failed = false;
        for repository in repositories {
            if repository.archived || repository.fork {
                sqlx::query("UPDATE delivery_sources SET enabled=FALSE WHERE git=$1")
                    .bind(&repository.clone_url)
                    .execute(&state.store.pool)
                    .await?;
                continue;
            }
            let state = state.clone();
            let client = client.clone();
            tasks.spawn(async move {
                let result = async {
                    if request_marker(&client, &repository.full_name, &repository.default_branch)
                        .await?
                    {
                        inspect(
                            &state,
                            &client,
                            &repository.full_name,
                            &repository.clone_url,
                            &repository.default_branch,
                        )
                        .await?;
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                if let Err(error) = &result {
                    eprintln!("检查 {} 发布标记失败: {error:#}", repository.full_name);
                }
                result.is_ok()
            });
            if tasks.len() >= 8 {
                failed |= !matches!(tasks.join_next().await, Some(Ok(true)));
            }
        }
        while let Some(result) = tasks.join_next().await {
            failed |= !matches!(result, Ok(true));
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

async fn request_marker(client: &Client, repository: &str, branch: &str) -> Result<bool> {
    let response = request(
        client,
        &format!("repos/{repository}/contents/aio-delivery.toml"),
    )
    .header("accept", "application/vnd.github.raw+json")
    .query(&[("ref", branch)])
    .send()
    .await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(false);
    }
    az_plugin_delivery::parse(&response.error_for_status()?.text().await?).map(|_| true)
}

pub(super) async fn run(state: RuntimeState) {
    let Some(config) = &state.config.delivery else {
        return;
    };
    let owner = &config.owner;
    if !owner
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        eprintln!("自动发现 owner 无效");
        return;
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("构建发现客户端");
    tokio::join!(discover(&state, &client, &owner), poll(&state, &client));
}

async fn discover(state: &RuntimeState, client: &Client, owner: &str) {
    let mut etags = std::collections::HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(
        state
            .config
            .delivery
            .as_ref()
            .expect("交付配置")
            .discovery_interval_seconds
            .max(1),
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(error) = scan(state, client, owner, &mut etags).await {
            eprintln!("扫描插件仓库失败: {error:#}");
        }
    }
}

async fn poll(state: &RuntimeState, client: &Client) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        state
            .config
            .delivery
            .as_ref()
            .expect("交付配置")
            .revision_interval_seconds
            .max(1),
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let result = async {
            let rows = sqlx::query("SELECT git,branch FROM delivery_sources WHERE enabled")
                .fetch_all(&state.store.pool)
                .await?;
            let mut tasks = tokio::task::JoinSet::new();
            for row in rows {
                let git: String = row.try_get("git")?;
                let branch: String = row.try_get("branch")?;
                let repo = git
                    .strip_prefix("https://github.com/")
                    .and_then(|s| s.strip_suffix(".git"))
                    .context("交付来源必须属于 GitHub")?
                    .to_owned();
                let state = state.clone();
                let client = client.clone();
                tasks.spawn(async move {
                    if let Err(error) = inspect(&state, &client, &repo, &git, &branch).await {
                        eprintln!("检查 {repo} 失败: {error:#}");
                    }
                });
                if tasks.len() >= 8 {
                    let _ = tasks.join_next().await;
                }
            }
            while tasks.join_next().await.is_some() {}
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            eprintln!("仓库发现: {error:#}");
        }
    }
}
