use super::{RuntimeState, http_error::RuntimeError, request_context::authenticate};
use crate::generated::worker_webview::model::ViewOwner;
use axum::http::{HeaderMap, header};

impl RuntimeState {
    /// 挂载凭据只能打开当前插件已获授权的本人设备，版本变化立即失效。
    pub(crate) async fn device_view_owner(&self, token: &str) -> Result<ViewOwner, RuntimeError> {
        let grant = self
            .frontend
            .get(token)
            .map_err(|_| RuntimeError::unauthorized("前端挂载已过期"))?;
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, grant.cookie.clone());
        let session = authenticate(self, &headers).await?;
        if !grant.page_id.starts_with("component:") {
            return Err(RuntimeError::forbidden("当前插件未声明设备视图授权"));
        }
        let grant = super::components::device_view_grant(self, &session, token).await?;
        Ok(ViewOwner {
            tenant: grant.tenant_id,
            user: grant.user_id,
            session: grant.session_id,
            source: grant.source_id,
            revision: grant.revision,
            mount: token.into(),
        })
    }
}
