use std::sync::Arc;

use super::super::component_catalog::ComponentInstallationCommit;
use super::{Components, execution::ComponentExecutor};
use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use uuid::Uuid;

struct PreviousExecution {
    executor: Arc<dyn ComponentExecutor>,
    bundle: Bundle,
}

impl Components {
    pub async fn install(&self, tenant: &str, git: &str, revision: Option<&str>) -> Result<()> {
        let _guard = self.mutations.lock().await;
        self.install_locked(tenant, git, revision, None).await
    }

    pub(super) async fn install_locked(
        &self,
        tenant: &str,
        git: &str,
        revision: Option<&str>,
        excluded: Option<&str>,
    ) -> Result<()> {
        let (source, bundle) = self
            .catalog
            .published(git, revision)
            .await?
            .context("插件尚未发布")?;
        ensure!(bundle.git == git, "发布来源与请求不匹配");
        if let Some(parent) = bundle
            .verify()?
            .manifest()
            .plugin
            .marketplace
            .as_ref()
            .and_then(|m| m.parent.as_deref())
        {
            ensure!(
                self.catalog.parent_enabled(tenant, parent).await?,
                "请先启用父插件: {parent}"
            );
        }
        let expected_generation = self
            .current_installation(tenant, source)
            .await?
            .and_then(|entry| entry.installation)
            .map(|installation| installation.generation);
        let executor = self.executor(&bundle)?;
        let previous = self.previous_execution(tenant, source).await?;
        let result = async {
            executor.activate(source, tenant, &bundle).await?;
            let commit = ComponentInstallationCommit {
                tenant: tenant.into(),
                source,
                digest: bundle.digest.clone(),
                excluded_digest: excluded.map(str::to_owned),
                expected_generation,
            };
            self.catalog.install(commit).await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            return match self
                .recover_execution(source, tenant, &executor, previous.as_ref())
                .await
            {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("安装失败且执行器恢复失败: {cleanup:#}")))
                }
            };
        }
        self.bind_execution(source, tenant, &bundle.digest, executor.clone())
            .await;
        if let Some(previous) = previous
            && previous.executor.as_ref().type_id() != executor.as_ref().type_id()
            && let Err(error) = previous.executor.deactivate(source, tenant).await
        {
            // 安装提交点已经完成，旧实例清理失败不能伪装成新版本未激活。
            eprintln!("清理旧执行器实例失败 ({source}, {tenant}): {error:#}");
        }
        Ok(())
    }

    async fn previous_execution(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<PreviousExecution>> {
        let Some(bundle) = self.enabled_bundle(tenant, source).await? else {
            return Ok(None);
        };
        let active = self.active.read().await;
        let Some(active) = active.get(&(source, tenant.into())) else {
            return Ok(None);
        };
        ensure!(
            active.digest == bundle.digest,
            "插件活动版本与安装记录不匹配"
        );
        let executor = active.executor.clone();
        Ok(Some(PreviousExecution { executor, bundle }))
    }

    async fn recover_execution(
        &self,
        source: Uuid,
        tenant: &str,
        candidate: &Arc<dyn ComponentExecutor>,
        previous: Option<&PreviousExecution>,
    ) -> Result<()> {
        let mut errors = Vec::new();
        if previous.is_none_or(|previous| {
            previous.executor.as_ref().type_id() != candidate.as_ref().type_id()
        }) && let Err(error) = candidate.deactivate(source, tenant).await
        {
            errors.push(format!("清理候选失败: {error:#}"));
        }
        if let Some(previous) = previous
            && let Err(error) = previous
                .executor
                .restore(source, tenant, &previous.bundle)
                .await
        {
            errors.push(format!("恢复旧实例失败: {error:#}"));
        }
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }

    pub async fn change(&self, tenant: &str, source: Uuid, action: &str) -> Result<()> {
        let _guard = self.mutations.lock().await;
        if action == "enable" {
            // 重新启用只恢复租户选择的版本，不能撤销回滚或隐式升级。
            let entry = self
                .current_installation(tenant, source)
                .await?
                .context("当前租户未安装插件")?;
            let installation = entry.installation.context("安装记录缺失")?;
            let git = entry.source.git;
            let digest = installation.digest;
            let excluded = installation.excluded_digest;
            return self
                .install_locked(tenant, &git, Some(&digest), excluded.as_deref())
                .await;
        }
        if action == "rollback" {
            let selection = self.catalog.rollback_target(tenant, source).await?;
            return self
                .install_locked(
                    tenant,
                    &selection.git,
                    Some(&selection.digest),
                    selection.excluded_digest.as_deref(),
                )
                .await;
        }
        ensure!(action == "disable" || action == "uninstall", "未知管理操作");
        self.require_no_children(tenant, source).await?;
        self.installed_bundle(tenant, source)
            .await?
            .context("当前租户未安装插件")?;
        let installation = self
            .current_installation(tenant, source)
            .await?
            .and_then(|entry| entry.installation)
            .context("当前租户未安装插件")?;
        let previous = self.previous_execution(tenant, source).await?;
        let result = async {
            if let Some(previous) = &previous {
                previous.executor.deactivate(source, tenant).await?;
            }
            self.catalog
                .remove(
                    tenant,
                    source,
                    installation.generation,
                    action == "uninstall",
                )
                .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let Some(previous) = previous.as_ref() else {
                return Err(error);
            };
            return match self
                .recover_execution(source, tenant, &previous.executor, Some(previous))
                .await
            {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("停用失败且执行器恢复失败: {cleanup:#}")))
                }
            };
        }
        self.active.write().await.remove(&(source, tenant.into()));
        Ok(())
    }
}
