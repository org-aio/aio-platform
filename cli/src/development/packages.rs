use super::package_content::Package;
use anyhow::{Result, ensure};
use az_plugin_development::{DevConfiguration, LockedPlugin, PublishedRelease};
use reqwest::{Url, blocking::Client, header};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct Packages {
    cache: PathBuf,
    origin: Url,
    client: Client,
    offline: bool,
    pub releases: BTreeMap<(String, String), PublishedRelease>,
}

impl Packages {
    pub fn new(root: &Path, offline: bool) -> Result<Self> {
        let mut origin = Url::parse(
            &std::env::var("AIO_DEV_CATALOG_URL")
                .unwrap_or_else(|_| crate::repository_plugin::DEFAULT_PUBLISH_URL.into()),
        )?;
        ensure!(
            (origin.scheme() == "https"
                || (origin.scheme() == "http"
                    && origin.host_str().is_some_and(|host| host == "localhost"
                        || host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback()))))
                && origin.username().is_empty()
                && origin.password().is_none(),
            "开发发布目录必须是 HTTPS 或本地地址"
        );
        origin.set_path("/");
        origin.set_query(None);
        origin.set_fragment(None);
        let mut headers = header::HeaderMap::new();
        if let Ok(session) = std::env::var("AIO_DEV_CATALOG_SESSION") {
            let mut value = header::HeaderValue::from_str(&format!("aio_session={session}"))?;
            value.set_sensitive(true);
            headers.insert(header::COOKIE, value);
        }
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60))
            .build()?;
        let cache = root.join(".aio/dev/packages");
        std::fs::create_dir_all(&cache)?;
        Ok(Self {
            cache,
            origin,
            client,
            offline,
            releases: BTreeMap::new(),
        })
    }

    pub fn candidates(
        &mut self,
        git: &str,
        locked: Option<&LockedPlugin>,
    ) -> Result<Vec<PublishedRelease>> {
        let releases = if let Some(locked) = locked.filter(|locked| locked.package_digest.is_some())
        {
            let digest = locked.package_digest.as_deref().unwrap();
            let package = self.package(git, digest)?;
            let metadata = package.metadata()?;
            ensure!(
                locked.version.as_deref() == Some(metadata.version.as_str())
                    && locked.source_sha.as_deref() == Some(metadata.source_sha.as_str()),
                "已锁定包的来源 SHA 或版本不匹配"
            );
            vec![metadata]
        } else {
            ensure!(
                !self.offline,
                "依赖 {git} 未缓存已发布版本；先联网准备，未发布依赖请使用 --with <路径>"
            );
            let response = self
                .client
                .get(self.origin.join("api/runtime/releases")?)
                .query(&[("git", git)])
                .send()?;
            ensure!(
                response.status() != reqwest::StatusCode::UNAUTHORIZED,
                "发布目录需要登录，请一次性配置 AIO_DEV_CATALOG_SESSION；未发布依赖请使用 --with"
            );
            let mut bytes = Vec::new();
            response
                .error_for_status()?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 16 * 1024 * 1024, "发布目录响应过大");
            serde_json::from_slice::<Vec<PublishedRelease>>(&bytes)?
        };
        ensure!(
            !releases.is_empty(),
            "依赖 {git} 尚未发布；请使用 --with <路径>"
        );
        for release in &releases {
            ensure!(
                release.git == git
                    && release.source_sha.len() == 40
                    && release.source_sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "发布目录返回无效来源"
            );
            self.releases
                .insert((git.into(), release.version.clone()), release.clone());
        }
        Ok(releases)
    }

    fn package(&self, git: &str, digest: &str) -> Result<Package> {
        ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "锁定包摘要无效"
        );
        let path = self.cache.join(format!("{digest}.aio-plugin"));
        let bytes = if path.is_file() {
            std::fs::read(&path)?
        } else {
            ensure!(!self.offline, "锁定依赖尚未缓存，需联网准备或使用 --with");
            let response = self
                .client
                .get(
                    self.origin
                        .join(&format!("api/runtime/packages/{digest}"))?,
                )
                .send()?
                .error_for_status()?;
            let mut bytes = Vec::new();
            response
                .take(az_plugin_bundle::MAX_ENCODED_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() <= az_plugin_bundle::MAX_ENCODED_BYTES,
                "依赖包超过配额"
            );
            bytes
        };
        let package = Package::decode(&bytes)?;
        let metadata = package.metadata()?;
        ensure!(
            metadata.git == git && metadata.digest == digest,
            "下载的依赖包来源或摘要不匹配"
        );
        if !path.is_file() {
            std::fs::write(path, bytes)?;
        }
        Ok(package)
    }

    pub fn workspace(&self, release: &PublishedRelease) -> Result<(PathBuf, DevConfiguration)> {
        let package = self.package(&release.git, &release.digest)?;
        let actual = package.metadata()?;
        ensure!(
            actual.version == release.version
                && actual.source_sha == release.source_sha
                && actual.manifest == release.manifest
                && actual.abi == release.abi,
            "依赖包与目录记录不一致"
        );
        let root = self.cache.join(&release.digest);
        let staging = tempfile_directory(&self.cache)?;
        let config = package.extract(&staging.path, &root)?;
        let digest = |path: &Path| {
            az_plugin_development::artifact_digest(
                &path.join("aio-plugin.toml"),
                &path.join(&config.frontend.output),
                &path.join(&config.backend.output),
            )
        };
        if root.exists() {
            ensure!(
                digest(&root)? == digest(&staging.path)?,
                "缓存依赖已被修改；删除损坏的 .aio/dev/packages 版本目录后重试"
            );
        } else {
            std::fs::rename(&staging.path, &root)?;
        }
        Ok((root, config))
    }
}

struct Staging {
    path: PathBuf,
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn tempfile_directory(cache: &Path) -> Result<Staging> {
    let path = cache.join(format!(".extract-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&path)?;
    Ok(Staging { path })
}

#[cfg(test)]
#[path = "packages_tests.rs"]
mod tests;
