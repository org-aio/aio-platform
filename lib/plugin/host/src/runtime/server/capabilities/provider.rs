use std::{any::Any, sync::Arc};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::InvocationScope;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::identity::SessionContext;

/// 宿主能力的副作用约定；幂等写操作必须以业务标识拒绝不同输入的重复请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffect {
    Read,
    IdempotentWrite,
    /// 外部写操作不保证幂等；适配器不得自动重试。
    Write,
}

/// 跨语言能力契约。路径用于业务路由，原生提供者仍按具体 TypeId 校验身份。
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCapability {
    pub path: String,
    pub description: String,
    pub effect: CapabilityEffect,
    pub input_schema: schemars::Schema,
    pub output_schema: schemars::Schema,
}

/// 只由运行时创建的调用，身份与插件授权来自真实请求，不读取脚本 context。
pub struct CapabilityRequest {
    pub path: String,
    pub input: Value,
    pub(super) scope: InvocationScope,
    pub(super) deadline: std::time::Instant,
}

impl CapabilityRequest {
    /// 整条调用链的截止时间，远端连接及请求均须沿用，不能为子调用重新计时。
    pub fn deadline(&self) -> std::time::Instant {
        self.deadline
    }

    pub fn scope(&self) -> &InvocationScope {
        &self.scope
    }

    /// 用户资源能力必须存在已验证的交互会话；租户后台调用不能借用用户身份。
    pub fn session(&self) -> Result<SessionContext> {
        let context = &self.scope.context;
        let session = context
            .session_id
            .as_deref()
            .context("宿主能力需要用户会话")?;
        ensure!(
            !session.is_empty() && !session.starts_with("service:"),
            "宿主能力需要交互会话"
        );
        Ok(SessionContext {
            session_id: session.into(),
            tenant_id: context.tenant_id.clone().context("宿主能力缺少租户")?,
            user_id: context.user_id.clone().context("宿主能力缺少用户")?,
            account: String::new(),
            display_name: String::new(),
            tenant_label: String::new(),
            permissions: vec![],
        })
    }
}

/// 原生服务或远端协议适配器的扩展点。不得脱离调用另起无人管理的副作用任务；
/// Future 被丢弃时停止等待，非幂等写操作不得自动重试。
#[async_trait::async_trait]
pub trait CapabilityProvider: Any + Send + Sync {
    fn capabilities(&self) -> Vec<RuntimeCapability>;
    /// 动态提供者返回不可变的执行快照；一次增强调用及其子调用始终持有同一快照。
    /// 默认实现表示静态提供者，其能力必须在初始化时完整声明。
    fn snapshot(&self) -> Option<Arc<dyn CapabilityProvider>> {
        None
    }
    /// 租户资源是否已装配；这是可用性检查，不代替运行时的显式能力授权。
    fn available(&self, _path: &str, _tenant: &str) -> bool {
        true
    }
    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value>;
}
