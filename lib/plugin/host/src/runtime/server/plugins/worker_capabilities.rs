use std::{any::TypeId, sync::Arc};

use anyhow::Result;
use az_plugin_core::{Plugin, PluginFuture};
use serde::Deserialize;
use serde_json::Value;

use super::{
    CapabilityEffect, CapabilityProvider, CapabilityRequest, CapabilityServicePlugin,
    RuntimeAssembly, RuntimeCapability,
};
use crate::generated::worker::{
    WorkerService,
    model::{SubmitTask, Task, Worker},
};

/// 将已装配的设备服务提供给增强脚本，不创建第二套设备、任务或授权存储。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct WorkerCapabilitiesPlugin;

impl Plugin<RuntimeAssembly> for WorkerCapabilitiesPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<CapabilityServicePlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn WorkerService>();
            assembly.services.add::<WorkerCapabilities>();
            Ok(())
        })
    }
}

#[dill::component]
#[dill::interface(dyn CapabilityProvider)]
#[dill::scope(dill::Singleton)]
struct WorkerCapabilities {
    workers: Arc<dyn WorkerService>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TaskId {
    id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct TaskSubmission {
    id: String,
    worker_id: String,
    capability: String,
    input: Value,
}

#[async_trait::async_trait]
impl CapabilityProvider for WorkerCapabilities {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        [
            ("/v1/workers/list", "列出当前用户的设备；输入为空对象", CapabilityEffect::Read),
            ("/v1/workers/tasks/get", "查询当前用户的任务；输入为 id", CapabilityEffect::Read),
            ("/v1/workers/tasks/submit", "提交当前用户设备已启用的任务；输入为 id、worker_id、capability、input，id 为幂等 UUID", CapabilityEffect::IdempotentWrite),
            ("/v1/workers/tasks/cancel", "取消当前用户的任务；输入为 id，重复取消保留终态", CapabilityEffect::IdempotentWrite),
        ].into_iter().map(|(path, description, effect)| {
            let input_schema = match path {
                "/v1/workers/list" => schemars::schema_for!(Empty),
                "/v1/workers/tasks/submit" => schemars::schema_for!(TaskSubmission),
                _ => schemars::schema_for!(TaskId),
            };
            let output_schema = if path == "/v1/workers/list" { schemars::schema_for!(Vec<Worker>) } else { schemars::schema_for!(Task) };
            RuntimeCapability { path: path.into(), description: description.into(), effect, input_schema, output_schema }
        }).collect()
    }

    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value> {
        let session = request.session()?;
        match request.path.as_str() {
            "/v1/workers/list" => {
                serde_json::from_value::<Empty>(request.input.clone())?;
                Ok(serde_json::to_value(self.workers.list(&session).await?)?)
            }
            "/v1/workers/tasks/get" | "/v1/workers/tasks/cancel" => {
                let input: TaskId = serde_json::from_value(request.input.clone())?;
                uuid::Uuid::parse_str(&input.id)?;
                let task = if request.path.ends_with("/cancel") {
                    self.workers.cancel_task(&session, &input.id).await?
                } else {
                    self.workers.task(&session, &input.id).await?
                };
                Ok(serde_json::to_value(task)?)
            }
            "/v1/workers/tasks/submit" => {
                let input: TaskSubmission = serde_json::from_value(request.input.clone())?;
                let task = SubmitTask {
                    id: input.id,
                    worker_id: input.worker_id,
                    capability: input.capability,
                    input: input.input,
                };
                Ok(serde_json::to_value(
                    self.workers.enqueue(&session, task).await?,
                )?)
            }
            _ => anyhow::bail!("设备能力路径不存在"),
        }
    }
}
