use super::super::component_catalog::{ComponentCatalogEntry, ComponentPublication};
use super::{Components, model::Description};
use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use uuid::Uuid;

impl Components {
    pub async fn contains_source(&self, source: &str) -> Result<bool> {
        let Ok(source) = Uuid::parse_str(source) else {
            return Ok(false);
        };
        self.catalog.contains_source(source).await
    }

    pub async fn publish(&self, bundle: Bundle, readme: &str) -> Result<Uuid> {
        self.publish_checked(bundle, readme, None).await
    }

    pub async fn publish_delivery(&self, bundle: Bundle, readme: &str, job: i64) -> Result<Uuid> {
        self.publish_checked(bundle, readme, Some(job)).await
    }

    async fn publish_checked(
        &self,
        bundle: Bundle,
        readme: &str,
        job: Option<i64>,
    ) -> Result<Uuid> {
        let _guard = self.mutations.lock().await;
        self.publish_locked(bundle, readme, job, true).await
    }

    pub(super) async fn publish_locked(
        &self,
        bundle: Bundle,
        readme: &str,
        job: Option<i64>,
        replace_publication: bool,
    ) -> Result<Uuid> {
        ensure!(readme.len() <= 512 * 1024, "README 超过限制");
        let verified = bundle.verify()?;
        let metadata = verified
            .manifest()
            .plugin
            .marketplace
            .as_ref()
            .context("发布包必须声明 plugin.marketplace")?;
        let existing = self.catalog.source(&bundle.git).await?;
        if let Some(existing) = &existing {
            ensure!(
                existing.parent == metadata.parent,
                "已发布来源不能改变父插件归属"
            );
        }
        let source = existing
            .map(|record| record.id)
            .unwrap_or_else(Uuid::new_v4);
        let mut parent = metadata.parent.clone();
        let mut ancestors = std::collections::HashSet::from([bundle.git.clone()]);
        while let Some(git) = parent {
            ensure!(
                ancestors.len() < 1024 && ancestors.insert(git.clone()),
                "父子插件关系形成循环或超过限制"
            );
            parent = self
                .catalog
                .source(&git)
                .await?
                .and_then(|record| record.parent);
        }
        let description = self.validate(source, &bundle).await?;
        let publication = ComponentPublication {
            source,
            bundle,
            description,
            readme: readme.into(),
            delivery_job: job,
            replace_publication,
        };
        self.catalog.publish(publication).await?;
        Ok(source)
    }

    pub async fn require_no_children(&self, tenant: &str, source: Uuid) -> Result<()> {
        let children = self.catalog.children(tenant, source).await?;
        ensure!(
            children.is_empty(),
            "请先卸载子插件: {}",
            children.join("、")
        );
        Ok(())
    }

    // 替代目录也不能返回另一个来源、租户或版本的安装记录。
    pub(super) async fn current_installation(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<ComponentCatalogEntry>> {
        let entry = self.catalog.installation(tenant, source).await?;
        if let Some(entry) = &entry {
            let installation = entry.installation.as_ref().context("目录缺少安装记录")?;
            ensure!(
                entry.source.id == source
                    && installation.tenant == tenant
                    && installation.digest == entry.version.digest,
                "目录安装归属或版本不匹配"
            );
        }
        Ok(entry)
    }

    pub async fn page_count(&self, tenant: &str, source: Uuid) -> Result<usize> {
        Ok(self.description(tenant, source).await?.2.pages.len())
    }

    pub(super) async fn description(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<(String, String, Description)> {
        if tenant == "development"
            && let Some(instance) = self.development.read().await.get(&source)
        {
            return Ok((
                instance.bundle.digest().into(),
                instance.generation.clone(),
                instance.description.clone(),
            ));
        }
        let entry = self
            .current_installation(tenant, source)
            .await?
            .context("当前租户未启用插件")?;
        let installation = entry.installation.context("目录缺少安装记录")?;
        ensure!(installation.enabled, "当前租户未启用插件");
        Ok((
            installation.digest,
            installation.generation.to_string(),
            entry.version.description,
        ))
    }
}
