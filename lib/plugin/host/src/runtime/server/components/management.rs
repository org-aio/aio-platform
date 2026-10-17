//! Wasm、Sidecar 与挂载前端共享部署信任、活动版本和实时交互授权。

use anyhow::{Context, Result, ensure};
use az_plugin_contract::{
    InvocationScope,
    transport::{Request, Response},
};

use super::Components;
use crate::{identity::SessionContext, runtime::server::plugins::ManagementAccess};

impl Components {
    pub(super) async fn manage(
        &self,
        scope: &InvocationScope,
        request: Request,
        session: SessionContext,
        executing: bool,
    ) -> Result<Response> {
        let state = self.management_state(scope, &session).await?;
        super::super::management_dispatch::invoke(
            state,
            scope.clone(),
            self.services.clone(),
            executing,
            request,
        )
        .await
    }

    // 前端文件传输与交互操作复用同一信任边界，不通过 JSON 装载整包内容。
    pub(super) async fn management_state(
        &self,
        scope: &InvocationScope,
        session: &SessionContext,
    ) -> Result<super::super::RuntimeState> {
        self.services.management_session(scope)?;
        ensure!(scope.grants.management, "插件未获管理能力");
        let bundle = self
            .bundle(scope.source_id.parse()?, &session.tenant_id)
            .await?;
        ensure!(
            bundle.digest() == scope.revision && bundle.manifest().plugin.capabilities.management,
            "管理插件活动版本已变化"
        );
        self.validate_authority(&bundle)?;
        ensure!(
            self.host_permissions(
                &session.tenant_id,
                &session.user_id,
                &session.session_id,
                &["plugin:manage".into()]
            )
            .await?,
            "当前用户无插件管理权限"
        );
        // 只从本代容器解析管理能力，旧进程或旧挂载不能继承下一代新增授权。
        let access = self
            .runtime_services
            .upgrade()
            .get_one::<ManagementAccess>()
            .context("宿主未开放管理能力")?;
        access.runtime()
    }
}
