use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use az_plugin_runtime::bindings::aio::plugin::transport::{Request, Response};
use std::sync::Arc;
use uuid::Uuid;

use super::Components;

impl Components {
    pub(super) async fn installed_bundle(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<Bundle>> {
        let archive: Option<Vec<u8>> = sqlx::query_scalar("SELECT v.archive FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1 AND i.source_id=$2").bind(tenant).bind(source).fetch_optional(&self.pool).await?;
        archive.map(|bytes| Bundle::decode(&bytes)).transpose()
    }

    pub(super) async fn install_process(
        &self,
        tenant: &str,
        source: Uuid,
        bundle: Bundle,
        excluded: Option<&str>,
    ) -> Result<()> {
        let enabled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations WHERE tenant_id=$1 AND source_id=$2 AND enabled)")
            .bind(tenant).bind(source).fetch_one(&self.pool).await?;
        let previous = if enabled {
            self.installed_bundle(tenant, source).await?
        } else {
            None
        };
        let result = async {
            self.processes.activate(source, tenant, &bundle).await?;
            let mut tx = self.pool.begin().await?;
            self.save_installation(&mut tx, tenant, source, &bundle.digest, excluded)
                .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            self.processes.stop(source, tenant).await?;
            if let Some(previous) = previous {
                self.processes.activate(source, tenant, &previous).await?;
            }
        }
        result
    }

    pub(super) async fn change_process(
        &self,
        tenant: &str,
        source: Uuid,
        action: &str,
        previous: Bundle,
    ) -> Result<()> {
        self.processes.stop(source, tenant).await?;
        let sql = if action == "uninstall" {
            "DELETE FROM component_installations WHERE source_id=$1 AND tenant_id=$2"
        } else {
            "UPDATE component_installations SET enabled=false,generation=gen_random_uuid() WHERE source_id=$1 AND tenant_id=$2"
        };
        if let Err(error) = sqlx::query(sql)
            .bind(source)
            .bind(tenant)
            .execute(&self.pool)
            .await
        {
            self.processes.activate(source, tenant, &previous).await?;
            return Err(error.into());
        }
        Ok(())
    }

    pub(super) async fn bundle(&self, source: Uuid, tenant: &str) -> Result<Arc<VerifiedBundle>> {
        if tenant == "development"
            && let Some(instance) = self.development.read().await.get(&source)
        {
            return Ok(instance.bundle.clone());
        }
        let (digest, _, description) = self.description(tenant, source).await?;
        let bundle = if description.process {
            self.processes
                .bundle(source, tenant)
                .await
                .context("process 未恢复")?
        } else {
            self.slot(source, tenant)
                .await?
                .snapshot()
                .await?
                .context("Component 未恢复")?
                .bundle
        };
        ensure!(bundle.digest() == digest, "插件活动版本已变化");
        Ok(bundle)
    }

    pub(super) async fn handle(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response> {
        let local = if tenant == "development" {
            self.development
                .read()
                .await
                .get(&source)
                .map(|instance| instance.slot.clone())
        } else {
            None
        };
        if let Some(slot) = local {
            return if let Some(slot) = slot {
                slot.handle(digest, request, context).await
            } else {
                self.processes
                    .handle(source, tenant, digest, request, context)
                    .await
            };
        }
        let (current, _, description) = self.description(tenant, source).await?;
        ensure!(digest == current, "插件活动版本已变化");
        if description.process {
            self.processes
                .handle(source, tenant, digest, request, context)
                .await
        } else {
            let slot = self.slot(source, tenant).await?;
            let retry = (request.clone(), context.clone());
            match slot.handle(digest, request, context).await {
                Ok(response) => Ok(response),
                Err(error) => {
                    slot.unload().await;
                    if format!("{error:#}").contains("实例已失效，必须重新创建") {
                        // 该错误发生在插件执行前，可安全重建后重试当前请求。
                        let slot = self.slot(source, tenant).await?;
                        let (request, context) = retry;
                        return slot.handle(digest, request, context).await;
                    }
                    // 超时、取消或业务失败的结果可能不确定，只清槽，不重放当前写入。
                    Err(error)
                }
            }
        }
    }
}
