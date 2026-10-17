//! 首次安装只追加版本与安装意图，不改变市场最新版本或租户之后的管理选择。

use std::{collections::HashSet, io::Read};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, MAX_BUNDLE_BYTES, MAX_ENCODED_BYTES};

use super::{Components, dispatch::validate_manifest};
use crate::runtime::server::plugins::BootstrapComponent;

struct PreparedComponent {
    bundle: Bundle,
    parent: Option<String>,
    tenants: Vec<String>,
}

pub(in crate::runtime::server) struct PreparedComponents(Vec<PreparedComponent>);

impl PreparedComponents {
    pub(in crate::runtime::server) fn load(declarations: Vec<BootstrapComponent>) -> Result<Self> {
        ensure!(declarations.len() <= 64, "首次引导最多允许 64 个整包");
        let mut pending = Vec::new();
        let mut sources = HashSet::new();
        let mut remaining = MAX_BUNDLE_BYTES * 4;
        for declaration in declarations {
            let component = prepare(declaration)?;
            ensure!(
                sources.insert(component.bundle.git.clone()),
                "首次引导来源重复"
            );
            let size = component
                .bundle
                .files
                .iter()
                .map(|(path, value)| path.len() + value.len())
                .sum::<usize>()
                + component.bundle.manifest.len();
            remaining = remaining
                .checked_sub(size)
                .context("首次引导整批解包超过 256 MiB")?;
            pending.push(component);
        }
        // 拓扑顺序只使用整包 Git 业务身份，不将其用于原生插件注册。
        let mut ordered = Vec::new();
        while !pending.is_empty() {
            let index = pending
                .iter()
                .position(|component| {
                    component.parent.as_ref().is_none_or(|parent| {
                        !pending
                            .iter()
                            .any(|candidate| &candidate.bundle.git == parent)
                    })
                })
                .context("首次引导父子插件关系形成循环")?;
            ordered.push(pending.remove(index));
        }
        Ok(Self(ordered))
    }
}

fn prepare(declaration: BootstrapComponent) -> Result<PreparedComponent> {
    ensure!(
        declaration.digest.len() == 64
            && declaration
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "首次引导必须固定规范 SHA256 摘要"
    );
    ensure!(
        !declaration.tenants.is_empty() && declaration.tenants.len() <= 128,
        "首次引导必须声明 1 至 128 个租户"
    );
    let mut tenants = HashSet::new();
    for tenant in &declaration.tenants {
        ensure!(
            !tenant.is_empty()
                && tenant.len() <= 128
                && tenant.trim() == tenant
                && !tenant.chars().any(char::is_control)
                && tenants.insert(tenant),
            "首次引导租户无效或重复"
        );
    }
    let metadata = std::fs::symlink_metadata(&declaration.path)
        .with_context(|| format!("读取首次引导整包失败: {}", declaration.path.display()))?;
    ensure!(metadata.is_file(), "首次引导整包必须是普通文件");
    ensure!(
        metadata.len() <= MAX_ENCODED_BYTES as u64,
        "首次引导整包超过限制"
    );
    let file = std::fs::File::open(&declaration.path)?;
    let mut bytes = Vec::new();
    file.take(MAX_ENCODED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let bundle = Bundle::decode(&bytes)?;
    let verified = bundle.verify()?;
    ensure!(
        verified.digest() == declaration.digest,
        "首次引导整包摘要不匹配"
    );
    validate_manifest(&verified)?;
    let metadata = verified
        .manifest()
        .plugin
        .marketplace
        .as_ref()
        .context("首次引导整包必须声明 plugin.marketplace")?;
    Ok(PreparedComponent {
        parent: metadata.parent.clone(),
        bundle,
        tenants: declaration.tenants,
    })
}

impl Components {
    pub(in crate::runtime::server) async fn bootstrap(
        &self,
        prepared: &PreparedComponents,
    ) -> Result<()> {
        let _guard = self.mutations.lock().await;
        let mut pending = Vec::new();
        // 整批候选发布校验成功后才开始安装；缺少执行器暂缓，歧义和实际失败仍暴露。
        for component in &prepared.0 {
            let available = self
                .executors
                .matching(&component.bundle.verify()?)?
                .is_some();
            let mut tenants = Vec::new();
            for tenant in &component.tenants {
                let initialized: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM component_initializations WHERE tenant_id=$1 AND git=$2)",
                ).bind(tenant).bind(&component.bundle.git).fetch_one(&self.pool).await?;
                if !initialized {
                    tenants.push(tenant);
                }
            }
            if tenants.is_empty() {
                continue;
            }
            if !available {
                eprintln!("首次引导暂缓，缺少执行器: {}", component.bundle.git);
                continue;
            }
            self.publish_locked(component.bundle.clone(), "", None, false)
                .await?;
            pending.push((component, tenants));
        }
        for (component, tenants) in pending {
            for tenant in tenants {
                if let Some(parent) = &component.parent
                    && !self.bootstrap_parent_ready(tenant, parent).await?
                {
                    eprintln!(
                        "首次引导暂缓，父插件未启用: {} ({tenant})",
                        component.bundle.git
                    );
                    continue;
                }
                self.install_locked(
                    tenant,
                    &component.bundle.git,
                    Some(&component.bundle.digest),
                    None,
                )
                .await
                .with_context(|| {
                    format!("首次引导安装失败: {} ({tenant})", component.bundle.git)
                })?;
            }
        }
        Ok(())
    }

    async fn bootstrap_parent_ready(&self, tenant: &str, parent: &str) -> Result<bool> {
        let installation: Option<(uuid::Uuid, String, bool)> = sqlx::query_as(
            "SELECT i.source_id,i.digest,i.enabled FROM component_installations i JOIN component_sources s ON s.id=i.source_id WHERE i.tenant_id=$1 AND s.git=$2",
        ).bind(tenant).bind(parent).fetch_optional(&self.pool).await?;
        if let Some((source, digest, enabled)) = installation {
            return Ok(enabled && self.is_active(source, tenant, &digest).await);
        }
        self.parent_enabled(tenant, parent).await
    }
}
