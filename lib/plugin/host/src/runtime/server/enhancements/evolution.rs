use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use az_plugin_enhancement::Package;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::capabilities::{
    CapabilityEffect, CapabilityProvider, CapabilityRequest, RuntimeCapability,
};
use super::{
    Enhancements,
    model::{Activation, Event, Head, Observation},
};
use crate::identity::IdentityProvider;

/// 将已有版本服务接到能力层；生成与演进策略由普通脚本或外部插件实现。
pub(in crate::runtime::server) struct EvolutionCapabilities {
    pub enhancements: Arc<Enhancements>,
    pub identity: Arc<dyn IdentityProvider>,
    pub authorization: Arc<dyn crate::authorization::AuthorizationProvider>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RevisionRequest {
    revision: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObservationRequest {
    revision: String,
    generation: i64,
}

#[derive(Serialize, schemars::JsonSchema)]
struct Proposed {
    revision: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CatalogRequest {
    paths: Vec<String>,
}

impl EvolutionCapabilities {
    async fn authorize(&self, request: &CapabilityRequest) -> Result<()> {
        let session = request.session()?;
        ensure!(
            std::time::Instant::now() < request.deadline(),
            "演进调用已超时"
        );
        ensure!(
            self.identity
                .member_active(&session.tenant_id, &session.user_id)
                .await?
                && self
                    .identity
                    .session_active(&session.session_id, &session.tenant_id, &session.user_id)
                    .await?,
            "演进调用会话已撤销"
        );
        let permissions = vec![
            "plugin:manage".to_owned(),
            format!("runtime:{}", request.path),
        ];
        ensure!(
            self.authorization
                .session_has_permissions(
                    &session.session_id,
                    &session.tenant_id,
                    &session.user_id,
                    &permissions
                )
                .await?,
            "当前会话缺少管理权限或演进能力授权"
        );
        ensure!(
            std::time::Instant::now() < request.deadline(),
            "演进调用已超时"
        );
        Ok(())
    }

    fn check_grants(&self, package: &Package, request: &CapabilityRequest) -> Result<()> {
        for enhancement in &package.enhancements {
            for capability in &enhancement.capabilities {
                ensure!(
                    request.scope().grants.runtime.contains(capability),
                    "演进候选不能增加调用者未获准的宿主能力: {capability}"
                );
            }
        }
        Ok(())
    }
}

fn capability<I: schemars::JsonSchema, O: schemars::JsonSchema>(
    operation: &str,
    description: &str,
    effect: CapabilityEffect,
) -> RuntimeCapability {
    RuntimeCapability {
        path: format!("/v1/runtime/enhancements/{operation}"),
        description: description.into(),
        effect,
        input_schema: schemars::schema_for!(I),
        output_schema: schemars::schema_for!(O),
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for EvolutionCapabilities {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        use CapabilityEffect::{IdempotentWrite, Read, Write};
        vec![
            capability::<Empty, Head>("head", "读取当前租户的活动增强版本和代数", Read),
            capability::<RevisionRequest, Package>(
                "package",
                "读取当前租户的不可变候选源码与契约样例",
                Read,
            ),
            capability::<CatalogRequest, Vec<RuntimeCapability>>(
                "catalog",
                "按路径发现已装配且获准调用的宿主能力；最多 32 项",
                Read,
            ),
            capability::<Package, Proposed>(
                "propose",
                "编译并验证候选样例；保存不可变版本，不改变活动指针",
                IdempotentWrite,
            ),
            capability::<Activation, Head>(
                "activate",
                "通过旧契约校验和代数 CAS 后激活；revision=null 表示停用",
                Write,
            ),
            capability::<Activation, Head>(
                "rollback",
                "按代数 CAS 恢复当前租户曾激活的版本",
                Write,
            ),
            capability::<Empty, Vec<Event>>(
                "history",
                "读取最近 50 次激活、停用与回滚的提交审计",
                Read,
            ),
            capability::<ObservationRequest, Vec<Observation>>(
                "observe",
                "读取指定激活代数的调用计数；started-succeeded-failed 包含未完成、取消和进程中断，不等同于故障",
                Read,
            ),
        ]
    }

    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value> {
        self.authorize(request).await?;
        let session = request.session()?;
        let tenant = &session.tenant_id;
        let operation = request
            .path
            .strip_prefix("/v1/runtime/enhancements/")
            .context("演进能力路径不存在")?;
        match operation {
            "head" => {
                serde_json::from_value::<Empty>(request.input.clone())?;
                Ok(serde_json::to_value(self.enhancements.head(tenant).await?)?)
            }
            "package" => {
                let input: RevisionRequest = serde_json::from_value(request.input.clone())?;
                Ok(serde_json::to_value(
                    self.enhancements.package(tenant, &input.revision).await?,
                )?)
            }
            "observe" => {
                let input: ObservationRequest = serde_json::from_value(request.input.clone())?;
                Ok(serde_json::to_value(
                    self.enhancements
                        .observe(tenant, &input.revision, input.generation)
                        .await?,
                )?)
            }
            "catalog" => {
                let input: CatalogRequest = serde_json::from_value(request.input.clone())?;
                ensure!(input.paths.len() <= 32, "单次能力发现超过 32 项");
                for path in &input.paths {
                    az_plugin_contract::validate_enhancement_path(path)?;
                }
                let catalog = self.enhancements.runtime_catalog(tenant)?;
                let catalog: Vec<_> = catalog
                    .into_iter()
                    .filter(|entry| {
                        input.paths.contains(&entry.path)
                            && request.scope().grants.runtime.contains(&entry.path)
                    })
                    .collect();
                Ok(serde_json::to_value(catalog)?)
            }
            "propose" => {
                let package: Package = serde_json::from_value(request.input.clone())?;
                self.check_grants(&package, request)?;
                let revision = self
                    .enhancements
                    .propose(tenant, &session.user_id, package, self.authorize(request))
                    .await?;
                Ok(serde_json::to_value(Proposed { revision })?)
            }
            "activate" | "rollback" => {
                let input: Activation = serde_json::from_value(request.input.clone())?;
                if let Some(revision) = &input.revision {
                    let package = self.enhancements.package(tenant, revision).await?;
                    self.check_grants(&package, request)?;
                }
                let head = self
                    .enhancements
                    .activate(
                        tenant,
                        &session.user_id,
                        input,
                        operation == "rollback",
                        self.authorize(request),
                    )
                    .await?
                    .context("增强活动版本已变化，必须重新审阅")?;
                Ok(serde_json::to_value(head)?)
            }
            "history" => {
                serde_json::from_value::<Empty>(request.input.clone())?;
                Ok(serde_json::to_value(
                    self.enhancements.history(tenant).await?,
                )?)
            }
            _ => anyhow::bail!("演进能力路径不存在"),
        }
    }
}
