use super::super::{remote_access, repository};
use anyhow::{Context as _, Result, ensure};
use az_tool::registration::{Documentation, Metadata};
use std::path::Path;

pub(super) async fn load(cache_root: &Path, metadata: Metadata) -> Documentation {
    let mut doc = Documentation {
        metadata,
        ..Default::default()
    };
    if doc.metadata.git.is_empty() {
        return doc;
    }
    let staging = cache_root.join(format!("cli-readme-{}", uuid::Uuid::new_v4()));
    let result = read(&staging, &doc.metadata.git).await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    match result {
        Ok((readme, revision)) => {
            let git = &doc.metadata.git;
            doc.link_base = format!("{git}/blob/{revision}/");
            doc.image_base = format!("{git}/raw/{revision}/");
            if let Some(path) = git.strip_prefix("https://github.com/") {
                doc.image_base = format!("https://raw.githubusercontent.com/{path}/{revision}/");
            }
            if git.starts_with("https://gitlab.com/") {
                doc.link_base = format!("{git}/-/blob/{revision}/");
                doc.image_base = format!("{git}/-/raw/{revision}/");
            }
            doc.readme = readme;
        }
        Err(error) => doc.error = Some(format!("README 暂时无法读取：{error:#}")),
    }
    doc
}

async fn read(staging: &Path, git: &str) -> Result<(String, String)> {
    let git = format!("{git}.git");
    remote_access::validate_git(&git)?;
    let remote = remote_access::validate_public_remote(&git).await?;
    read_remote(staging, &git, &remote).await
}

pub(super) async fn read_remote(
    staging: &Path,
    git: &str,
    remote: &remote_access::RemoteResolution,
) -> Result<(String, String)> {
    remote_access::validate_git(git)?;
    ensure!(
        reqwest::Url::parse(git)?.host_str() == Some(remote.host.as_str()),
        "Git 解析主机不匹配"
    );
    ensure!(
        remote_access::is_public_ip(remote.socket.ip()),
        "Git 地址必须为公网 IP"
    );
    tokio::fs::create_dir_all(staging).await?;
    repository::run_git_bounded(Some(staging), &["init", "--quiet", "--bare"], staging).await?;
    let pin = remote.git_configuration();
    // 只拉取对象并读取文档，不检出工作区，不运行仓库脚本或钩子。
    repository::run_git_bounded(
        Some(staging),
        &[
            "-c",
            "http.followRedirects=false",
            "-c",
            &pin,
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "fetch",
            "--quiet",
            "--depth",
            "1",
            git,
            "HEAD",
        ],
        staging,
    )
    .await?;
    let revision = repository::git_output(staging, &["rev-parse", "FETCH_HEAD"]).await?;
    ensure!(
        revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
        "Git 提交无效"
    );
    let readme = read_blob(staging).await?;
    Ok((readme, revision))
}

pub(super) async fn read_blob(staging: &Path) -> Result<String> {
    let tree = repository::git_output(staging, &["ls-tree", "FETCH_HEAD"]).await?;
    let name = tree
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(kind, name)| {
            kind.starts_with("100")
                && ["readme.md", "readme.markdown", "readme"]
                    .contains(&name.to_ascii_lowercase().as_str())
        })
        .map(|(_, name)| name)
        .context("仓库根目录没有 README 文件")?;
    let object = format!("FETCH_HEAD:{name}");
    let size: usize = repository::git_output(staging, &["cat-file", "-s", &object])
        .await?
        .parse()?;
    ensure!(size <= 256 * 1024, "README 超过 256 KiB");
    repository::git_output(staging, &["cat-file", "blob", &object]).await
}
