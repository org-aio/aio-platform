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
        let rows = self.catalog.installations(None, true).await?;
        for row in rows {
            let source = row.source.id;
            let installation = row.installation.context("安装记录缺失")?;
            ensure!(installation.enabled, "恢复列表含停用安装");
            let tenant = installation.tenant;
            let digest = installation.digest;
            ensure!(row.version.digest == digest, "安装索引与版本不匹配");
            let archive = self
                .catalog
                .archive(&digest)
                .await?
                .context("安装整包缺失")?;
            let bundle = Bundle::decode(&archive)?;
            let verified = bundle.verify()?;
            ensure!(verified.digest() == digest, "安装记录与整包摘要不匹配");
            ensure!(bundle.git == row.source.git, "安装记录与整包来源不匹配");
            validate_manifest(&verified)?;
            if verified.manifest().plugin.capabilities.management
                && !self.trusted_management(&verified)?
            {
                continue;
            }
            // 只允许缺少实现时暂停；数据损坏、重复匹配和恢复错误仍阻止启动。
            let Some(executor) = self.executors.matching(&verified)? else {
                continue;
            };
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
        let verified = bundle.verify()?;
        self.validate_authority(&verified)?;
        self.executors.select(&verified)
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

    pub(super) async fn active_executor(
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

    pub(super) async fn is_active(&self, source: Uuid, tenant: &str, digest: &str) -> bool {
        self.active
            .read()
            .await
            .get(&(source, tenant.into()))
            .is_some_and(|active| active.digest == digest)
    }

    pub(super) async fn installation_state(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        enabled: bool,
    ) -> crate::runtime::PluginState {
        use crate::runtime::PluginState;
        if !enabled {
            PluginState::Disabled
        } else if self.is_active(source, tenant, digest).await {
            PluginState::Active
        } else {
            PluginState::Suspended
        }
    }

    pub(super) async fn validate(&self, source: Uuid, bundle: &Bundle) -> Result<Description> {
        let verified = bundle.verify()?;
        self.validate_authority(&verified)?;
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
        self.installation_bundle(tenant, source, false).await
    }

    pub(super) async fn enabled_bundle(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<Bundle>> {
        self.installation_bundle(tenant, source, true).await
    }

    async fn installation_bundle(
        &self,
        tenant: &str,
        source: Uuid,
        enabled_only: bool,
    ) -> Result<Option<Bundle>> {
        let Some(row) = self.current_installation(tenant, source).await? else {
            return Ok(None);
        };
        let installation = row.installation.context("安装记录缺失")?;
        if enabled_only && !installation.enabled {
            return Ok(None);
        }
        let bytes = self
            .catalog
            .archive(&installation.digest)
            .await?
            .context("安装整包缺失")?;
        let bundle = Bundle::decode(&bytes)?;
        ensure!(
            bundle.verify()?.digest() == installation.digest,
            "安装记录与整包摘要不匹配"
        );
        ensure!(bundle.git == row.source.git, "安装记录与整包来源不匹配");
        Ok(Some(bundle))
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
        let executor = self
            .authorized_executor(source, tenant, digest, &context)
            .await?;
        executor
            .handle(source, tenant, digest, request, context)
            .await
    }

    pub(super) async fn handle_stream(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<super::execution::StreamingResponse> {
        ensure!(
            matches!(request.method.as_str(), "GET" | "HEAD") && request.body.is_empty(),
            "流式入口仅允许读取"
        );
        let executor = self
            .authorized_executor(source, tenant, digest, &context)
            .await?;
        executor
            .handle_stream(source, tenant, digest, request, context)
            .await
    }

    async fn authorized_executor(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        context: &RequestContext,
    ) -> Result<Arc<dyn ComponentExecutor>> {
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
            ensure!(registered == *context, "交互请求归属不匹配");
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
        self.active_executor(source, tenant, digest).await
    }
}

// 发布与部署引导共用执行器之外的准入校验，缺少执行器也不能掩盖越权声明。
pub(super) fn validate_manifest(verified: &VerifiedBundle) -> Result<()> {
    let grants = &verified.manifest().plugin.capabilities;
    ensure!(!grants.identity_provider, "当前发布入口未开放身份提供能力");
    ensure!(
        !grants.management
            || verified
                .manifest()
                .plugin
                .required_permissions
                .iter()
                .any(|p| p == "plugin:manage"),
        "管理整包必须要求宿主 plugin:manage 权限"
    );
    ensure!(
        semver::VersionReq::parse(&verified.manifest().plugin.runtime.host_version)?
            .matches(&semver::Version::parse(env!("CARGO_PKG_VERSION"))?),
        "宿主版本不满足插件约束"
    );
    Ok(())
}

impl Components {
    pub(super) fn trusted_management(&self, bundle: &VerifiedBundle) -> Result<bool> {
        let services = self.runtime_services.upgrade();
        if services
            .builders_for::<super::super::plugins::ManagementAccess>()
            .count()
            == 0
        {
            return Ok(false);
        }
        Ok(services
            .get_one::<super::super::plugins::ManagementAccess>()?
            .trusts(bundle))
    }

    pub(super) fn validate_authority(&self, bundle: &VerifiedBundle) -> Result<()> {
        validate_manifest(bundle)?;
        ensure!(
            !bundle.manifest().plugin.capabilities.management || self.trusted_management(bundle)?,
            "宿主未信任此管理整包版本"
        );
        Ok(())
    }
}
