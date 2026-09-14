use crate::identity::SessionContext;
use anyhow::{Result, bail};
use async_trait::async_trait;
use az_plugin_contract::{InvocationScope, RequestContext};
use az_plugin_runtime::{
    HostServices,
    bindings::aio::plugin::transport::{Request, Response},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub(super) fn permission(source: impl std::fmt::Display, name: &str) -> String {
    format!("component:{source}:{name}")
}

#[derive(Default)]
pub(super) struct Services {
    requests: Mutex<HashMap<String, (RequestContext, Vec<String>)>>,
}
pub(super) struct Authorization {
    pub context: RequestContext,
    services: Arc<Services>,
}
impl Services {
    pub(super) fn interactive(&self, id: &str, tenant: &str, user: &str) -> Result<RequestContext> {
        let requests = self
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
        let (context, _) = requests
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("交互请求已结束"))?;
        anyhow::ensure!(
            context.tenant_id.as_deref() == Some(tenant)
                && context.user_id.as_deref() == Some(user)
                && context
                    .session_id
                    .as_deref()
                    .is_some_and(|s| !s.starts_with("service:")),
            "交互请求归属不匹配"
        );
        Ok(context.clone())
    }

    pub(super) fn background(
        self: &Arc<Self>,
        tenant: &str,
        user: &str,
        source: &str,
        permissions: Vec<String>,
    ) -> Result<Authorization> {
        let context = RequestContext {
            tenant_id: Some(tenant.into()),
            user_id: Some(user.into()),
            session_id: Some(format!("service:{source}")),
            request_id: uuid::Uuid::new_v4().to_string(),
        };
        self.requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?
            .insert(context.request_id.clone(), (context.clone(), permissions));
        Ok(Authorization {
            context,
            services: self.clone(),
        })
    }
    pub fn enter(self: &Arc<Self>, session: &SessionContext) -> Result<Authorization> {
        let context = RequestContext {
            tenant_id: Some(session.tenant_id.clone()),
            user_id: Some(session.user_id.clone()),
            session_id: Some(session.session_id.clone()),
            request_id: uuid::Uuid::new_v4().to_string(),
        };
        self.requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?
            .insert(
                context.request_id.clone(),
                (context.clone(), session.permissions.clone()),
            );
        Ok(Authorization {
            context,
            services: self.clone(),
        })
    }
}
impl Drop for Authorization {
    fn drop(&mut self) {
        if let Ok(mut requests) = self.services.requests.lock() {
            requests.remove(&self.context.request_id);
        }
    }
}
#[async_trait]
impl HostServices for Services {
    async fn authorize(&self, scope: &InvocationScope, permission: &str) -> Result<bool> {
        let requests = self
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
        Ok(requests
            .get(&scope.context.request_id)
            .is_some_and(|(context, permissions)| {
                let required = self::permission(&scope.source_id, permission);
                context == &scope.context && permissions.iter().any(|p| p == &required || p == "*")
            }))
    }
    async fn manage(&self, _: &InvocationScope, _: Request) -> Result<Response> {
        bail!("当前执行器未授予管理接口")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interactive_context_is_scoped_and_expires_with_the_request() -> Result<()> {
        let services = Arc::new(Services::default());
        let session = SessionContext {
            tenant_id: "tenant".into(),
            user_id: "user".into(),
            session_id: "session".into(),
            account: "account".into(),
            display_name: "User".into(),
            tenant_label: "Tenant".into(),
            permissions: vec![],
        };
        let request = services.enter(&session)?;
        let id = request.context.request_id.clone();
        assert!(services.interactive(&id, "tenant", "user").is_ok());
        assert!(services.interactive(&id, "other", "user").is_err());
        assert!(services.interactive(&id, "tenant", "other").is_err());
        drop(request);
        assert!(services.interactive(&id, "tenant", "user").is_err());
        let worker = services.background("tenant", "user", "parent", vec![])?;
        assert!(
            services
                .interactive(&worker.context.request_id, "tenant", "user")
                .is_err()
        );
        Ok(())
    }
    #[tokio::test]
    async fn permissions_cannot_cross_plugin_or_host_boundaries() -> Result<()> {
        let services = Services::default();
        let context = RequestContext {
            request_id: "request".into(),
            ..Default::default()
        };
        services.requests.lock().unwrap().insert(
            context.request_id.clone(),
            (
                context.clone(),
                vec![permission("first", "plugin:manage"), "host:read".into()],
            ),
        );
        let mut scope = InvocationScope {
            source_id: "first".into(),
            revision: "revision".into(),
            context,
            grants: Default::default(),
        };
        assert!(services.authorize(&scope, "plugin:manage").await?);
        assert!(!services.authorize(&scope, "host:read").await?);
        scope.source_id = "second".into();
        assert!(!services.authorize(&scope, "plugin:manage").await?);
        Ok(())
    }
}
