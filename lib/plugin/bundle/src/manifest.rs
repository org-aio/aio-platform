use anyhow::{Result, ensure};
use az_plugin_contract::CapabilityGrants;
use serde::{Deserialize, Serialize};

use crate::validate_relative_path;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub plugin: ComponentManifest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_page: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<az_plugin_manifest::RepositoryDependency>,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub marketplace: Option<MarketplaceManifest>,
    pub runtime: RuntimeManifest,
    pub frontend: FrontendManifest,
    pub database: Option<DatabaseManifest>,
    #[serde(default)]
    pub capabilities: CapabilityGrants,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceManifest {
    pub title: String,
    pub summary: String,
    pub license: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub parent: Option<String>,
    pub parent_title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub artifact: String,
    pub host_version: String,
    pub process: Option<ProcessManifest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessManifest {
    pub image: String,
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_endpoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worker_capabilities: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontendManifest {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseManifest {
    pub migrations: String,
}

impl BundleManifest {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(
            text.len() <= crate::model::MAX_MANIFEST_BYTES,
            "清单超过配额"
        );
        let manifest: Self = toml::from_str(text)?;
        ensure!(
            manifest.schema_version == az_plugin_contract::ABI_VERSION,
            "只接受 v2 清单"
        );
        let plugin = &manifest.plugin;
        if let Some(page) = &plugin.settings_page {
            ensure!(
                !page.is_empty()
                    && page.len() <= 128
                    && page
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "设置页面 ID 无效"
            );
        }
        az_plugin_manifest::validate_dependencies(&plugin.dependencies)?;
        ensure!(
            plugin.permissions.len() <= 64
                && plugin.permissions.iter().all(|p| !p.is_empty()
                    && p.len() <= 128
                    && p != "*"
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))),
            "插件权限声明无效"
        );
        if let Some(marketplace) = &plugin.marketplace {
            ensure!(
                !marketplace.title.trim().is_empty() && marketplace.title.len() <= 256,
                "市场标题无效"
            );
            ensure!(
                marketplace.summary.len() <= 2048
                    && !marketplace.license.is_empty()
                    && marketplace.license.len() <= 128,
                "市场简介或许可证无效"
            );
            ensure!(
                marketplace.tags.len() <= 16
                    && marketplace
                        .tags
                        .iter()
                        .all(|tag| !tag.is_empty() && tag.len() <= 64),
                "市场标签超过限制"
            );
            if let Some(parent) = &marketplace.parent {
                ensure!(
                    parent.starts_with("https://github.com/")
                        && parent.ends_with(".git")
                        && parent.len() <= 256
                        && parent
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b":/._-".contains(&b)),
                    "父插件必须是规范 GitHub 仓库地址"
                );
            }
            if let Some(title) = &marketplace.parent_title {
                ensure!(
                    marketplace.parent.is_some() && !title.trim().is_empty() && title.len() <= 256,
                    "父插件标题必须非空且声明父仓库"
                );
            }
        }
        validate_relative_path(&plugin.runtime.artifact)?;
        if let Some(process) = &plugin.runtime.process {
            ensure!(
                process.worker_capabilities.len() <= 2
                    && process
                        .worker_capabilities
                        .iter()
                        .all(|capability| matches!(
                            capability.as_str(),
                            "desktop.open-app" | "skills.sync"
                        )),
                "process 设备能力未开放"
            );
            let (image, digest) = process
                .image
                .split_once("@sha256:")
                .or_else(|| {
                    process
                        .image
                        .strip_prefix("sha256:")
                        .map(|digest| ("sha256", digest))
                })
                .ok_or_else(|| anyhow::anyhow!("process 镜像必须锁定 SHA-256"))?;
            ensure!(
                !image.is_empty()
                    && image.len() <= 256
                    && image
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"/._:-".contains(&b))
                    && digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "process 镜像无效"
            );
            ensure!(
                process.endpoints.len() <= 16
                    && process.http_endpoints.len() <= 16
                    && process.services.len() <= 16,
                "process 授权超过配额"
            );
            for endpoint in &process.endpoints {
                az_plugin_contract::process::model_endpoint(endpoint)
                    .map_err(anyhow::Error::msg)?;
            }
            for endpoint in &process.http_endpoints {
                let url = az_plugin_contract::process::model_endpoint(endpoint)
                    .map_err(anyhow::Error::msg)?;
                ensure!(url.scheme() == "https", "第三方 HTTP 出站仅接受 HTTPS");
            }
            for service in &process.services {
                ensure!(
                    service.starts_with("https://github.com/")
                        && service.ends_with(".git")
                        && service.len() <= 256
                        && service
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b":/._-".contains(&b)),
                    "服务授权必须是规范 GitHub 来源"
                );
            }
        } else {
            ensure!(
                plugin.runtime.artifact.ends_with(".wasm"),
                "Component 包需要 .wasm 后端产物"
            );
        }
        semver::VersionReq::parse(&plugin.runtime.host_version)?;
        validate_relative_path(&plugin.frontend.path)?;
        ensure!(
            plugin.runtime.artifact != plugin.frontend.path
                && !plugin
                    .runtime
                    .artifact
                    .starts_with(&format!("{}/", plugin.frontend.path)),
            "后端不能放在公开前端目录"
        );
        if let Some(database) = &plugin.database {
            ensure!(plugin.capabilities.database, "迁移必须申请数据库能力");
            validate_relative_path(&database.migrations)?;
            for path in [&plugin.frontend.path, &plugin.runtime.artifact] {
                ensure!(
                    path != &database.migrations
                        && !path.starts_with(&format!("{}/", database.migrations))
                        && !database.migrations.starts_with(&format!("{path}/")),
                    "数据库迁移目录与其他产物重叠"
                );
            }
        }
        Ok(manifest)
    }
}
