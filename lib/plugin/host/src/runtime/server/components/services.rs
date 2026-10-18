use crate::identity::SessionContext;
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use az_plugin_contract::transport::{Request, Response};
use az_plugin_contract::{InvocationScope, RequestContext};
use az_plugin_runtime::HostServices;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub(super) fn permission(source: impl std::fmt::Display, name: &str) -> String {
    format!("component:{source}:{name}")
}

#[derive(Default)]
pub(in crate::runtime::server) struct Services {
    enhancements: Option<Arc<super::super::enhancements::Enhancements>>,
    components: OnceLock<Weak<super::Components>>,
    requests: Mutex<HashMap<String, RequestAuthorization>>,
}

struct RequestAuthorization {
    context: RequestContext,
    source: String,
    permissions: Vec<String>,
    parent: Option<String>,
    session: Option<SessionContext>,
    executing: bool,
}

pub(super) struct Authorization {
    pub context: RequestContext,
    services: Arc<Services>,
}

// 派生请求不能比原交互请求活得更久；来源只能由宿主在登记时指定。
fn active_request<'a>(
    requests: &'a HashMap<String, RequestAuthorization>,
    id: &str,
) -> Result<&'a RequestAuthorization> {
    let request = requests.get(id).context("交互请求已结束")?;
    let mut current = request;
    while let Some(parent) = &current.parent {
        current = requests.get(parent).context("上游交互请求已结束")?;
    }
    Ok(request)
}

impl Services {
    pub(super) fn bind_components(&self, components: Weak<super::Components>) -> Result<()> {
        ensure!(
            self.components.set(components).is_ok(),
            "组件服务不能重复绑定"
        );
        Ok(())
    }

    pub(super) fn with_enhancements(
        enhancements: Option<Arc<super::super::enhancements::Enhancements>>,
    ) -> Self {
        Self {
            enhancements,
            ..Default::default()
        }
    }

    pub(super) fn interactive(
        &self,
        id: &str,
        tenant: &str,
        source: &str,
    ) -> Result<RequestContext> {
        let requests = self
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
        let request = active_request(&requests, id)?;
        let context = &request.context;
        ensure!(
            request.source == source
                && context.tenant_id.as_deref() == Some(tenant)
                && context
                    .user_id
                    .as_deref()
                    .is_some_and(|user| !user.is_empty())
                && context.session_id.as_deref().is_some_and(|session| {
                    !session.is_empty() && !session.starts_with("service:")
                }),
            "交互请求归属不匹配"
        );
        Ok(context.clone())
    }

    // broker 已校验目标依赖后，为目标插件派生独立上下文，权限仍来自原用户。
    #[cfg(any(test, feature = "sidecar"))]
    pub(super) fn delegate(
        self: &Arc<Self>,
        context: &RequestContext,
        caller: &str,
        target: &str,
    ) -> Result<Authorization> {
        let (permissions, session) = {
            let requests = self
                .requests
                .lock()
                .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
            let request = active_request(&requests, &context.request_id)?;
            ensure!(
                request.source == caller && request.context == *context,
                "交互请求归属不匹配"
            );
            (request.permissions.clone(), request.session.clone())
        };
        let mut delegated = context.clone();
        delegated.request_id = uuid::Uuid::new_v4().to_string();
        self.register(
            delegated,
            target,
            permissions,
            Some(context.request_id.clone()),
            session,
            true,
        )
    }

    #[cfg(any(test, feature = "sidecar"))]
    pub(super) fn background(
        self: &Arc<Self>,
        tenant: &str,
        user: &str,
        caller: &str,
        target: &str,
        permissions: Vec<String>,
    ) -> Result<Authorization> {
        let context = RequestContext {
            tenant_id: Some(tenant.into()),
            user_id: Some(user.into()),
            session_id: Some(format!("service:{caller}")),
            request_id: uuid::Uuid::new_v4().to_string(),
            permissions: permissions.clone(),
        };
        self.register(context, target, permissions, None, None, true)
    }

    pub(super) fn enter(
        self: &Arc<Self>,
        session: &SessionContext,
        source: &str,
    ) -> Result<Authorization> {
        let context = RequestContext {
            tenant_id: Some(session.tenant_id.clone()),
            user_id: Some(session.user_id.clone()),
            session_id: Some(session.session_id.clone()),
            request_id: uuid::Uuid::new_v4().to_string(),
            permissions: session.permissions.clone(),
        };
        self.register(
            context,
            source,
            session.permissions.clone(),
            None,
            Some(session.clone()),
            true,
        )
    }

    // 前端直接调用管理契约时没有占用插件执行器，可以安全管理该插件自身。
    pub(super) fn enter_frontend_management(
        self: &Arc<Self>,
        session: &SessionContext,
        source: &str,
    ) -> Result<Authorization> {
        let authorization = self.enter(session, source)?;
        self.requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?
            .get_mut(&authorization.context.request_id)
            .context("交互请求未登记")?
            .executing = false;
        Ok(authorization)
    }

    pub(in crate::runtime::server) fn management_session(
        &self,
        scope: &InvocationScope,
    ) -> Result<(SessionContext, bool)> {
        let requests = self
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
        let request = active_request(&requests, &scope.context.request_id)?;
        ensure!(
            request.source == scope.source_id && request.context == scope.context,
            "管理请求归属不匹配"
        );
        // 普通组件不能借跨插件调用把用户身份转交给受信任代理以提升自身能力。
        ensure!(request.parent.is_none(), "管理能力不能通过跨插件调用转授");
        let session = request
            .session
            .clone()
            .context("管理调用必须来自真实交互会话")?;
        ensure!(
            !session.session_id.starts_with("service:"),
            "后台调用不能管理宿主"
        );
        Ok((session, request.executing))
    }

    fn register(
        self: &Arc<Self>,
        mut context: RequestContext,
        source: &str,
        permissions: Vec<String>,
        parent: Option<String>,
        session: Option<SessionContext>,
        executing: bool,
    ) -> Result<Authorization> {
        let mut requests = self
            .requests
            .lock()
            .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
        if let Some(parent) = &parent {
            active_request(&requests, parent)?;
        }
        // 权限快照由宿主在此唯一登记点写入：Sidecar 通过上下文读取它，授权校验也
        // 比较同一字段，两边不会出现“入口放行、执行拒绝”或相反的分歧。
        if context.permissions != permissions {
            context.permissions.clone_from(&permissions);
        }
        let request = RequestAuthorization {
            context: context.clone(),
            source: source.into(),
            permissions,
            parent,
            session,
            executing,
        };
        requests.insert(context.request_id.clone(), request);
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
        let Ok(request) = active_request(&requests, &scope.context.request_id) else {
            return Ok(false);
        };
        let required = self::permission(&scope.source_id, permission);
        Ok(request.source == scope.source_id
            && request.context == scope.context
            && request
                .permissions
                .iter()
                .any(|p| p == &required || p == "*"))
    }

    async fn manage(&self, scope: &InvocationScope, request: Request) -> Result<Response> {
        let (session, executing) = self.management_session(scope)?;
        let components = self
            .components
            .get()
            .and_then(Weak::upgrade)
            .context("组件宿主已关闭")?;
        components.manage(scope, request, session, executing).await
    }

    async fn enhance(
        &self,
        scope: &InvocationScope,
        request: az_plugin_contract::EnhancementRequest,
    ) -> Result<az_plugin_contract::EnhancementResponse> {
        let mut scope = scope.clone();
        {
            let requests = self
                .requests
                .lock()
                .map_err(|_| anyhow::anyhow!("请求权限锁损坏"))?;
            let authorization = active_request(&requests, &scope.context.request_id)?;
            ensure!(
                authorization.source == scope.source_id && authorization.context == scope.context,
                "宿主能力请求归属不匹配"
            );
            scope.grants.runtime.retain(|path| {
                authorization
                    .permissions
                    .iter()
                    .any(|permission| permission == &format!("runtime:{path}"))
            });
        }
        self.enhancements
            .as_ref()
            .context("增强服务未绑定")?
            .invoke(&scope, request, None)
            .await
    }
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
