use std::{
    env,
    io::Cursor,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{
    artifact_path, frontend_files, read_manifest, validate_declared_pages, validate_frontend_pages,
    validate_host_compatibility, validate_page_definitions, validate_repository,
};
use flate2::read::GzDecoder;
use serde_json::Value;
use tar::Archive;
use tokio::process::Command;

use super::publication_validation::{ensure_publish_capabilities, ensure_publish_runtime};
#[cfg(test)]
use super::remote_access::is_public_ip;
use super::remote_access::{RemoteResolution, git_timeout, validate_git, validate_public_remote};
use crate::runtime::{PageDefinition, PluginRuntime};

const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ARCHIVE_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const DEFAULT_CACHE_BYTES: u64 = 1024 * 1024 * 1024;
const DEFAULT_CACHE_ENTRIES: usize = 100_000;
const DEFAULT_CACHE_REVISIONS: usize = 512;

pub struct DiscoveredPlugin {
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub manifest: Value,
    pub pages: Vec<PageDefinition>,
    pub artifact: String,
}

pub struct RepositoryInstaller {
    pub(super) cache_root: PathBuf,
    pub(super) cache_operations: tokio::sync::Mutex<()>,
}

impl RepositoryInstaller {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            cache_operations: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn discover(&self, git: &str, revision: Option<&str>) -> Result<DiscoveredPlugin> {
        ensure!(
            !revision.is_some_and(is_package_revision),
            "插件包内容版本必须从插件中心安装，不能作为 Git ref 拉取"
        );
        validate_git(git)?;
        let remote = validate_public_remote(git).await?;
        let _cache_guard = self.cache_operations.lock().await;
        tokio::fs::create_dir_all(&self.cache_root)
            .await
            .context("创建插件缓存目录失败")?;
        let staging = self
            .cache_root
            .join(format!("staging-{}", uuid::Uuid::new_v4()));
        let result = async {
            let (checkout_root, resolved) =
                checkout_revision(git, revision, &staging, Some(&remote)).await?;
            ensure!(
                resolved.len() == 40 && resolved.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Git 未解析出完整提交 SHA"
            );
            let manifest = read_manifest(&checkout_root)?;
            validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
            let validation_root = checkout_root.clone();
            let report = tokio::task::spawn_blocking(move || validate_repository(&validation_root))
                .await
                .context("等待插件 artifact 校验失败")??;
            let runtime = manifest
                .plugin
                .runtime
                .as_ref()
                .context("公网宿主只接受声明 plugin.runtime 的插件")?;
            let runtime_kind = runtime.kind;
            if runtime_kind == PluginRuntime::WasmComponent {
                ensure!(
                    manifest.plugin.capabilities.network.is_empty()
                        && manifest.plugin.capabilities.filesystem.is_empty()
                        && !manifest.plugin.capabilities.database,
                    "当前 Wasm Component 宿主未授予网络、文件系统或数据库能力"
                );
            }
            let artifact = report.artifact.context("运行时插件缺少 artifact")?;
            let pages = match runtime_kind {
                PluginRuntime::PageDefinition => az_plugin_manifest::parse_page_definitions(
                    &tokio::fs::read(&artifact).await.with_context(|| {
                        format!("读取 PageDefinition 失败: {}", artifact.display())
                    })?,
                )
                .context("解析 PageDefinition 失败")?,
                PluginRuntime::WasmComponent => Vec::new(),
                PluginRuntime::Process => Vec::new(),
                PluginRuntime::RustSource => {
                    anyhow::bail!("rust-source 插件必须通过整套发布切换")
                }
            };
            if runtime_kind == PluginRuntime::PageDefinition {
                validate_page_definitions(&pages)?;
                validate_declared_pages(&manifest, &pages)?;
            }
            let final_directory = self.cache_root.join(&resolved);
            if final_directory.exists() {
                tokio::fs::remove_dir_all(&staging).await?;
            } else {
                self.ensure_cache_quota(true).await?;
                tokio::fs::rename(&checkout_root, &final_directory)
                    .await
                    .context("原子发布插件缓存失败")?;
            }
            Ok(DiscoveredPlugin {
                source_id: published_source_id(git),
                git: git.to_owned(),
                revision: resolved,
                runtime: runtime_kind,
                manifest: serde_json::to_value(&manifest.plugin)?,
                pages,
                artifact: runtime.artifact.clone(),
            })
        }
        .await;
        finish_with_cleanup(result, &staging).await
    }

    pub async fn validate_published(&self, git: &str, revision: &str) -> Result<DiscoveredPlugin> {
        validate_git(git)?;
        ensure!(
            is_artifact_revision(revision),
            "插件版本必须是完整 Git SHA 或插件包 SHA-256"
        );
        let root = self.cache_root.join(revision);
        let manifest = read_manifest(&root)?;
        validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
        let runtime = manifest
            .plugin
            .runtime
            .as_ref()
            .context("发布插件缺少 plugin.runtime")?;
        ensure_publish_runtime(runtime.kind)?;
        ensure_publish_capabilities(&manifest)?;
        let validation_root = root.clone();
        let report = tokio::task::spawn_blocking(move || validate_repository(&validation_root))
            .await
            .context("等待发布 artifact 校验失败")??;
        let pages = if runtime.kind == PluginRuntime::PageDefinition {
            az_plugin_manifest::parse_page_definitions(
                &tokio::fs::read(artifact_path(&root, &runtime.artifact)?).await?,
            )
            .context("解析已发布 PageDefinition 失败")?
        } else {
            Vec::new()
        };
        Ok(DiscoveredPlugin {
            source_id: published_source_id(git),
            git: git.to_owned(),
            revision: revision.to_owned(),
            runtime: report.runtime,
            manifest: serde_json::to_value(&manifest.plugin)?,
            pages,
            artifact: runtime.artifact.clone(),
        })
    }

    pub fn artifact(&self, revision: &str, relative: &str) -> Result<PathBuf> {
        ensure!(
            is_artifact_revision(revision),
            "插件 revision 必须是完整 Git SHA 或插件包 SHA-256"
        );
        artifact_path(&self.cache_root.join(revision), relative)
    }

    pub fn validate_pages(&self, revision: &str, pages: &[PageDefinition]) -> Result<()> {
        ensure!(
            is_artifact_revision(revision),
            "插件 revision 必须是完整 Git SHA 或插件包 SHA-256"
        );
        let root = self.cache_root.join(revision);
        let manifest = read_manifest(&root)?;
        validate_page_definitions(pages)?;
        validate_declared_pages(&manifest, pages)?;
        let files = frontend_files(&root, &manifest)?;
        validate_frontend_pages(&manifest, pages, files.keys().map(String::as_str))
    }

    pub(super) async fn ensure_cache_quota(&self, adds_revision: bool) -> Result<()> {
        let root = self.cache_root.clone();
        let byte_limit = environment_limit("AIO_PLUGIN_CACHE_MAX_BYTES", DEFAULT_CACHE_BYTES);
        let revision_limit = environment_limit(
            "AIO_PLUGIN_CACHE_MAX_REVISIONS",
            DEFAULT_CACHE_REVISIONS as u64,
        );
        let revision_limit = usize::try_from(revision_limit).unwrap_or(usize::MAX);
        let entry_limit =
            environment_limit("AIO_PLUGIN_CACHE_MAX_ENTRIES", DEFAULT_CACHE_ENTRIES as u64);
        let entry_limit = usize::try_from(entry_limit).unwrap_or(usize::MAX);
        tokio::task::spawn_blocking(move || {
            validate_cache_quota(
                &root,
                byte_limit,
                revision_limit,
                entry_limit,
                adds_revision,
            )
        })
        .await
        .context("等待插件缓存配额校验失败")?
    }
}

fn environment_limit(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn validate_cache_quota(
    root: &Path,
    byte_limit: u64,
    revision_limit: usize,
    entry_limit: usize,
    adds_revision: bool,
) -> Result<()> {
    let mut directories = vec![root.to_owned()];
    let mut bytes = 0_u64;
    let mut entries = 0_usize;
    let revisions = std::fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.file_name().to_str().is_some_and(is_artifact_revision)
        })
        .count();
    ensure!(
        revisions.saturating_add(usize::from(adds_revision)) <= revision_limit,
        "插件缓存版本数量超过配额"
    );
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            entries = entries.checked_add(1).context("插件缓存条目数溢出")?;
            ensure!(entries <= entry_limit, "插件缓存条目数超过配额");
            bytes = bytes
                .checked_add(metadata.len())
                .context("插件缓存大小溢出")?;
            ensure!(bytes <= byte_limit, "插件缓存总大小超过配额");
            if metadata.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}

pub(super) async fn finish_with_cleanup<T>(result: Result<T>, directory: &Path) -> Result<T> {
    let cleanup = async {
        if tokio::fs::try_exists(directory).await? {
            tokio::fs::remove_dir_all(directory).await?;
        }
        Ok::<_, std::io::Error>(())
    }
    .await;
    match result {
        Ok(value) => {
            cleanup.context("清理插件临时目录失败")?;
            Ok(value)
        }
        Err(error) => {
            let _ = cleanup;
            Err(error)
        }
    }
}

fn github_archive(git: &str, revision: Option<&str>) -> Result<Option<(reqwest::Url, String)>> {
    let Some(revision) = revision.filter(|value| is_full_revision(value)) else {
        return Ok(None);
    };
    let source = reqwest::Url::parse(git).context("插件 Git 地址无效")?;
    if source.host_str() != Some("github.com") {
        return Ok(None);
    }
    let segments = source
        .path_segments()
        .context("GitHub 仓库地址缺少路径")?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() != 2 {
        return Ok(None);
    }
    let repository = segments[1]
        .strip_suffix(".git")
        .context("GitHub 仓库地址必须以 .git 结尾")?;
    ensure!(
        [segments[0], repository].into_iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        }),
        "GitHub 仓库路径包含无效字符"
    );
    let archive = reqwest::Url::parse(&format!(
        "https://codeload.github.com/{}/{repository}/tar.gz/{revision}",
        segments[0]
    ))
    .context("构造 GitHub 提交归档地址失败")?;
    Ok(Some((archive, revision.to_owned())))
}

pub(super) fn is_full_revision(revision: &str) -> bool {
    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn is_package_revision(revision: &str) -> bool {
    revision.len() == 64
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn is_artifact_revision(revision: &str) -> bool {
    is_full_revision(revision) || is_package_revision(revision)
}

pub(super) fn published_source_id(git: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, git.as_bytes()).to_string()
}

async fn checkout_revision(
    git: &str,
    revision: Option<&str>,
    staging: &Path,
    remote: Option<&RemoteResolution>,
) -> Result<(PathBuf, String)> {
    if let Some((_, requested)) = github_archive(git, revision)? {
        let remote = remote.context("GitHub 提交解析缺少固定公网地址")?;
        let resolved = resolve_remote_revision(git, &requested, staging, remote).await?;
        ensure!(
            resolved.eq_ignore_ascii_case(&requested),
            "GitHub ref 没有解析为请求中的完整提交 SHA"
        );
        let (archive, _) =
            github_archive(git, Some(&resolved))?.context("无法构造已解析 GitHub 提交归档")?;
        return Ok((download_archive(archive, staging).await?, resolved));
    }
    tokio::fs::create_dir_all(staging).await?;
    run_git(
        None,
        &[
            "init",
            "--quiet",
            staging.to_str().context("插件缓存路径不是 Unicode")?,
        ],
    )
    .await?;
    let checkout = revision.unwrap_or("HEAD");
    let pin = remote.map(RemoteResolution::git_configuration);
    let mut fetch = vec!["-c", "http.followRedirects=false"];
    if let Some(pin) = pin.as_deref() {
        fetch.extend(["-c", pin]);
    }
    fetch.extend(["fetch", "--quiet", "--depth", "1", git, checkout]);
    run_git_bounded(Some(staging), &fetch, staging).await?;
    run_git_bounded(
        Some(staging),
        &["checkout", "--quiet", "FETCH_HEAD"],
        staging,
    )
    .await?;
    let resolved = git_output(staging, &["rev-parse", "HEAD"]).await?;
    if revision.is_some_and(is_full_revision) {
        ensure!(
            revision.is_some_and(|requested| resolved.eq_ignore_ascii_case(requested)),
            "Git ref 没有解析为请求中的完整提交 SHA"
        );
    }
    tokio::fs::remove_dir_all(staging.join(".git")).await?;
    validate_directory_quota(staging).await?;
    Ok((staging.to_owned(), resolved))
}

async fn resolve_remote_revision(
    git: &str,
    revision: &str,
    staging: &Path,
    remote: &RemoteResolution,
) -> Result<String> {
    let repository = staging.join(".revision.git");
    tokio::fs::create_dir_all(staging).await?;
    run_git(
        None,
        &[
            "init",
            "--quiet",
            "--bare",
            repository
                .to_str()
                .context("插件提交解析路径不是 Unicode")?,
        ],
    )
    .await?;
    let pin = remote.git_configuration();
    run_git_bounded(
        Some(&repository),
        &[
            "-c",
            "http.followRedirects=false",
            "-c",
            &pin,
            "fetch",
            "--quiet",
            "--depth",
            "1",
            git,
            revision,
        ],
        &repository,
    )
    .await?;
    let resolved = git_output(&repository, &["rev-parse", "FETCH_HEAD"]).await?;
    tokio::fs::remove_dir_all(repository).await?;
    Ok(resolved)
}

async fn download_archive(source: reqwest::Url, staging: &Path) -> Result<PathBuf> {
    let remote = validate_public_remote(source.as_str()).await?;
    let client = reqwest::Client::builder()
        .connect_timeout(git_timeout())
        .timeout(git_timeout())
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&remote.host, remote.socket)
        .build()
        .context("创建 GitHub 提交归档客户端失败")?;
    let mut response = client
        .get(source)
        .send()
        .await
        .context("下载 GitHub 提交归档失败")?
        .error_for_status()
        .context("GitHub 提交归档返回失败状态")?;
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_ARCHIVE_BYTES),
        "GitHub 提交归档超过 32 MiB 配额"
    );
    let mut content = Vec::new();
    while let Some(chunk) = response.chunk().await.context("读取 GitHub 提交归档失败")? {
        ensure!(
            content.len().saturating_add(chunk.len()) <= MAX_ARCHIVE_BYTES as usize,
            "GitHub 提交归档超过 32 MiB 配额"
        );
        content.extend_from_slice(&chunk);
    }
    let staging = staging.to_owned();
    let extraction = staging.clone();
    tokio::task::spawn_blocking(move || extract_archive(content, &extraction))
        .await
        .context("等待 GitHub 提交归档解压失败")??;
    let mut entries = tokio::fs::read_dir(&staging)
        .await
        .context("读取 GitHub 提交归档目录失败")?;
    let root = entries.next_entry().await?.context("GitHub 提交归档为空")?;
    ensure!(
        root.file_type().await?.is_dir(),
        "GitHub 提交归档根不是目录"
    );
    ensure!(
        entries.next_entry().await?.is_none(),
        "GitHub 提交归档包含多个根目录"
    );
    Ok(root.path())
}

fn extract_archive(content: Vec<u8>, destination: &Path) -> Result<()> {
    let decoder = GzDecoder::new(Cursor::new(content));
    let mut archive = Archive::new(decoder);
    let mut expanded_bytes = 0_u64;
    for (index, entry) in archive.entries()?.enumerate() {
        ensure!(index < MAX_ARCHIVE_ENTRIES, "GitHub 提交归档条目数超过配额");
        let mut entry = entry.context("读取 GitHub 提交归档条目失败")?;
        expanded_bytes = expanded_bytes
            .checked_add(entry.header().size()?)
            .context("GitHub 提交归档展开大小溢出")?;
        ensure!(
            expanded_bytes <= MAX_ARCHIVE_EXPANDED_BYTES,
            "GitHub 提交归档展开后超过 64 MiB 配额"
        );
        ensure!(
            entry.unpack_in(destination)?,
            "GitHub 提交归档包含目标目录外路径"
        );
    }
    Ok(())
}

async fn validate_directory_quota(root: &Path) -> Result<()> {
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut directories = vec![root];
        let mut entries = 0_usize;
        let mut bytes = 0_u64;
        while let Some(directory) = directories.pop() {
            let directory_entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            for entry in directory_entries {
                let entry = entry?;
                let metadata = match std::fs::symlink_metadata(entry.path()) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                entries = entries.checked_add(1).context("Git 检出条目数溢出")?;
                ensure!(entries <= MAX_ARCHIVE_ENTRIES, "Git 检出条目数超过配额");
                if metadata.is_dir() {
                    directories.push(entry.path());
                } else {
                    bytes = bytes
                        .checked_add(metadata.len())
                        .context("Git 检出大小溢出")?;
                    ensure!(
                        bytes <= MAX_ARCHIVE_EXPANDED_BYTES,
                        "Git 检出内容超过 64 MiB 配额"
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("等待 Git 检出配额校验失败")?
}

async fn run_git(directory: Option<&Path>, arguments: &[&str]) -> Result<()> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let output = tokio::time::timeout(git_timeout(), command.output())
        .await
        .context("执行 Git 超时")??;
    ensure!(
        output.status.success(),
        "Git 失败: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

async fn run_git_bounded(
    directory: Option<&Path>,
    arguments: &[&str],
    quota_root: &Path,
) -> Result<()> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let mut child = command.spawn().context("启动受限 Git 操作失败")?;
    let deadline = tokio::time::sleep(git_timeout());
    tokio::pin!(deadline);
    let mut quota_check = tokio::time::interval(Duration::from_millis(50));
    quota_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            status = child.wait() => {
                ensure!(status?.success(), "受限 Git 操作失败");
                validate_directory_quota(quota_root).await?;
                return Ok(());
            }
            _ = quota_check.tick() => {
                if let Err(error) = validate_directory_quota(quota_root).await {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(error);
                }
            }
            _ = &mut deadline => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                anyhow::bail!("受限 Git 操作超时");
            }
        }
    }
}

async fn git_output(directory: &Path, arguments: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command
        .args(arguments)
        .current_dir(directory)
        .kill_on_drop(true);
    let output = tokio::time::timeout(git_timeout(), command.output())
        .await
        .context("执行 Git 超时")??;
    ensure!(
        output.status.success(),
        "Git 失败: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
