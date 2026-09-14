use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_contract::{CapabilityGrants, InvocationScope, RequestContext};
use tokio::sync::{Mutex, RwLock};

use crate::{
    ComponentEngine, ComponentInstance, InvocationResources,
    bindings::aio::plugin::{
        metadata::Description,
        transport::{Phase, Request, Response},
    },
};

pub struct ComponentSlot {
    source: uuid::Uuid,
    tenant: String,
    host_version: semver::Version,
    updates: Mutex<()>,
    state: RwLock<SlotState>,
}

#[derive(Default)]
struct SlotState {
    migrations: Option<Vec<(String, String)>>,
    active: Option<ActiveComponent>,
}

pub struct ReleaseSnapshot {
    pub bundle: Arc<VerifiedBundle>,
    pub description: Description,
}

struct ActiveComponent {
    bundle: Arc<VerifiedBundle>,
    description: Description,
    instance: Mutex<ComponentInstance>,
}

impl ComponentSlot {
    pub fn new(source: uuid::Uuid, tenant: String, host_version: semver::Version) -> Result<Self> {
        ensure!(!source.is_nil(), "来源 UUID 不能为空");
        ensure!(!tenant.trim().is_empty(), "实例必须绑定租户");
        Ok(Self {
            source,
            tenant,
            host_version,
            updates: Mutex::new(()),
            state: RwLock::new(SlotState::default()),
        })
    }

    pub async fn snapshot(&self) -> Option<ReleaseSnapshot> {
        self.state
            .read()
            .await
            .active
            .as_ref()
            .map(|active| ReleaseSnapshot {
                bundle: Arc::clone(&active.bundle),
                description: active.description.clone(),
            })
    }

    pub async fn replace(
        &self,
        engine: &ComponentEngine,
        bundle: Arc<VerifiedBundle>,
        grants: CapabilityGrants,
        resources: InvocationResources,
    ) -> Result<()> {
        let _update = self.updates.lock().await;
        let migrations: Vec<_> = bundle
            .migrations()
            .map(|(name, sql)| (name.to_owned(), sql.to_owned()))
            .collect();
        if let Some(previous) = &self.state.read().await.migrations {
            ensure!(
                previous == &migrations,
                "数据库迁移集合发生变化，需要受控维护迁移，不能直接在线替换"
            );
        }
        let requested = &bundle.manifest().plugin.capabilities;
        ensure!(
            (!requested.cryptography || grants.cryptography)
                && (!requested.database || grants.database)
                && (!requested.storage || grants.storage)
                && (!requested.management || grants.management)
                && (!requested.identity_provider || grants.identity_provider),
            "宿主未授予插件申请的全部能力"
        );
        ensure!(
            !requested.identity_provider,
            "身份提供者尚未接入此租户执行槽"
        );
        ensure!(
            !requested.database || resources.database.is_some(),
            "数据库能力未绑定已迁移的专属数据库"
        );
        ensure!(
            !requested.cryptography || resources.keyring.is_some(),
            "加密能力未绑定宿主密钥"
        );
        ensure!(
            !requested.storage || resources.storage.is_some(),
            "对象存储能力未绑定"
        );
        ensure!(
            !requested.management || resources.services.is_some(),
            "插件管理能力未绑定"
        );
        ensure!(
            semver::VersionReq::parse(&bundle.manifest().plugin.runtime.host_version)?
                .matches(&self.host_version),
            "宿主版本不满足插件约束"
        );
        let compiled = engine.compile(bundle.component()).await?;
        let mut candidate = engine
            .instantiate(
                &compiled,
                InvocationScope {
                    source_id: self.source.to_string(),
                    revision: bundle.digest().into(),
                    context: RequestContext {
                        tenant_id: Some(self.tenant.clone()),
                        ..Default::default()
                    },
                    // 宿主允许的能力仍须由当前整包申请，不能从旧版本继承额外授权。
                    grants: requested.clone(),
                },
                resources,
            )
            .await?;
        candidate.lifecycle(Phase::Prepare).await?;
        let description = candidate.describe().await?;
        for page in &description.pages {
            ensure!(
                bundle.frontend(&page.entry).is_some(),
                "页面入口不存在于当前整包: {}",
                page.entry
            );
        }
        candidate.health().await?;
        candidate.lifecycle(Phase::Activate).await?;
        candidate.health().await?;
        // 请求持有读锁到执行结束；写锁排空旧请求后一次性交换前端、描述和后端。
        let mut state = self.state.write().await;
        state.migrations = Some(migrations);
        let previous = state.active.replace(ActiveComponent {
            bundle,
            description,
            instance: Mutex::new(candidate),
        });
        drop(state);
        retire(previous).await;
        Ok(())
    }

    /// 仅资源变化时保留后端实例；能力、迁移和运行声明改变时交回完整替换流程。
    pub async fn refresh_frontend(&self, bundle: Arc<VerifiedBundle>) -> Result<bool> {
        let _update = self.updates.lock().await;
        let mut state = self.state.write().await;
        let Some(active) = state.active.as_mut() else {
            return Ok(false);
        };
        let execution_manifest = |bundle: &VerifiedBundle| -> Result<serde_json::Value> {
            let mut manifest = serde_json::to_value(&bundle.manifest().plugin)?;
            let object = manifest.as_object_mut().context("插件清单不是对象")?;
            object.remove("frontend");
            object.remove("marketplace");
            Ok(manifest)
        };
        if active.bundle.component() != bundle.component()
            || active.bundle.migrations().collect::<Vec<_>>()
                != bundle.migrations().collect::<Vec<_>>()
            || execution_manifest(&active.bundle)? != execution_manifest(&bundle)?
        {
            return Ok(false);
        }
        for page in &active.description.pages {
            ensure!(
                bundle.frontend(&page.entry).is_some(),
                "新版前端缺少页面入口"
            );
        }
        active
            .instance
            .lock()
            .await
            .rebind_revision(bundle.digest())?;
        active.bundle = bundle;
        Ok(true)
    }

    pub async fn handle(
        &self,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response> {
        ensure!(
            context.tenant_id.as_deref() == Some(&self.tenant),
            "调用租户不匹配"
        );
        let state = self.state.read().await;
        let active = state.active.as_ref().context("插件未激活")?;
        ensure!(
            active.bundle.digest() == digest,
            "页面版本已撤销，请重新挂载"
        );
        let response = active
            .instance
            .lock()
            .await
            .handle(request, context)
            .await?;
        Ok(response)
    }

    pub async fn deactivate(&self) {
        let _update = self.updates.lock().await;
        let previous = self.state.write().await.active.take();
        retire(previous).await;
    }
}

async fn retire(previous: Option<ActiveComponent>) {
    if let Some(previous) = previous {
        // 旧实例已不接收请求；清理失败也不能撤销已完成的切换。
        if let Err(error) = previous
            .instance
            .into_inner()
            .lifecycle(Phase::Deactivate)
            .await
        {
            tracing::warn!(revision = previous.bundle.digest(), %error, "停用旧实例的清理钩子失败，销毁实例");
        }
    }
}
