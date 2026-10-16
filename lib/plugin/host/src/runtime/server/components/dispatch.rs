use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use uuid::Uuid;

use super::{
    Components,
    execution::{ActiveExecution, ComponentExecutor, Request, Response},
    model::Description,
};

impl Components {
    pub async fn restore(&self) -> Result<()> {
        let rows = sqlx::query_as::<_, (Uuid, String, Vec<u8>)>(
            "SELECT i.source_id,i.tenant_id,v.archive FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.enabled",
        ).fetch_all(&self.pool).await?;
        for (source, tenant, archive) in rows {
            let bundle = Bundle::decode(&archive)?;
            let executor = self.executor(&bundle)?;
            executor.restore(source, &tenant, &bundle).await?;
            self.bind_execution(source, &tenant, &bundle.digest, executor)
                .await;
        }
        for executor in &self.executors.implementations {
            executor.reconcile().await?;
        }
        Ok(())
    }

    pub(super) fn executor(&self, bundle: &Bundle) -> Result<Arc<dyn ComponentExecutor>> {
        self.executors.select(&bundle.verify()?)
    }

    pub(super) async fn bind_execution(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        executor: Arc<dyn ComponentExecutor>,
    ) {
        let active = ActiveExecution {
            digest: digest.into(),
            executor,
        };
        self.active
            .write()
            .await
            .insert((source, tenant.into()), active);
    }

    async fn active_executor(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
    ) -> Result<Arc<dyn ComponentExecutor>> {
        let active = self.active.read().await;
        let active = active
            .get(&(source, tenant.into()))
            .context("插件执行实例未恢复")?;
        ensure!(active.digest == digest, "插件活动版本已变化");
        Ok(active.executor.clone())
    }

    pub(super) async fn validate(&self, source: Uuid, bundle: &Bundle) -> Result<Description> {
        let verified = bundle.verify()?;
        let grants = &verified.manifest().plugin.capabilities;
        ensure!(
            !grants.management && !grants.identity_provider,
            "当前发布入口未开放宿主管理或身份提供能力"
        );
        ensure!(
            semver::VersionReq::parse(&verified.manifest().plugin.runtime.host_version)?
                .matches(&semver::Version::parse(env!("CARGO_PKG_VERSION"))?),
            "宿主版本不满足插件约束"
        );
        self.executors
            .select(&verified)?
            .validate(source, bundle)
            .await
    }

    pub(super) async fn installed_bundle(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<Bundle>> {
        let archive: Option<Vec<u8>> = sqlx::query_scalar("SELECT v.archive FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1 AND i.source_id=$2")
            .bind(tenant).bind(source).fetch_optional(&self.pool).await?;
        archive.map(|bytes| Bundle::decode(&bytes)).transpose()
    }

    pub(super) async fn enabled_bundle(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<Bundle>> {
        let archive: Option<Vec<u8>> = sqlx::query_scalar("SELECT v.archive FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1 AND i.source_id=$2 AND i.enabled")
            .bind(tenant).bind(source).fetch_optional(&self.pool).await?;
        archive.map(|bytes| Bundle::decode(&bytes)).transpose()
    }

    pub(super) async fn bundle(&self, source: Uuid, tenant: &str) -> Result<Arc<VerifiedBundle>> {
        let (digest, _, _) = self.description(tenant, source).await?;
        let executor = self.active_executor(source, tenant, &digest).await?;
        let bundle = executor
            .bundle(source, tenant)
            .await?
            .context("插件执行实例未恢复")?;
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
        ensure!(
            context.tenant_id.as_deref() == Some(tenant),
            "调用租户不匹配"
        );
        let (current, _, _) = self.description(tenant, source).await?;
        ensure!(digest == current, "插件活动版本已变化");
        let required = self.required_permissions(tenant, source, digest).await?;
        if !required.is_empty() {
            // 所有执行器共用此门槛；仅伪造会话字段或后台用户标识不能获得交互授权。
            let registered =
                self.services
                    .interactive(&context.request_id, tenant, &source.to_string())?;
            ensure!(registered == context, "交互请求归属不匹配");
            ensure!(
                self.host_permissions(
                    tenant,
                    context.user_id.as_deref().unwrap_or_default(),
                    context.session_id.as_deref().unwrap_or_default(),
                    &required,
                )
                .await?,
                "当前用户无插件所需的宿主权限"
            );
        }
        let executor = self.active_executor(source, tenant, digest).await?;
        executor
            .handle(source, tenant, digest, request, context)
            .await
    }
}
