use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use super::super::plugins::{
    CapabilityProvider, CapabilityRequest, PluginTasks, RuntimeCapability,
};
use super::{
    configuration::{self, Configuration, Transport},
    grpc::Grpc,
    mcp::Mcp,
};

/// 同一原生 Provider 承载配置中的协议路由；业务路径和租户不冒充插件 TypeId。
pub(in crate::runtime::server) struct RemoteCapabilities {
    routes: BTreeMap<String, Route>,
    stopped: AtomicBool,
    tasks: Arc<PluginTasks>,
}

struct Route {
    capability: RuntimeCapability,
    input: jsonschema::Validator,
    output: jsonschema::Validator,
    tenants: BTreeMap<String, Binding>,
}

enum Binding {
    Mcp(Mcp),
    Grpc(Grpc),
}

impl RemoteCapabilities {
    pub async fn open(path: &Path, tasks: Arc<PluginTasks>) -> Result<Self> {
        let config: Configuration = serde_json::from_slice(&configuration::read(path).await?)?;
        ensure!(config.schema_version == 1, "不支持的远端能力配置版本");
        ensure!(
            !config.imports.is_empty() && config.imports.len() <= 64,
            "远端能力绑定数必须在 1 到 64 之间"
        );
        let mut routes = BTreeMap::<String, Route>::new();
        let mut bindings = HashSet::new();
        let mut pending = Vec::new();
        // 先校验整份配置的地址、凭据、schema 与重复绑定，避免发现本地错误前连接上游。
        for import in config.imports {
            ensure!(
                !import.tenant_id.trim().is_empty() && import.tenant_id != "*",
                "远端能力必须绑定明确租户"
            );
            az_plugin_contract::validate_enhancement_path(&import.capability.path)?;
            let auth = configuration::credential(import.credential_env.as_deref())?;
            configuration::endpoint(
                &import.endpoint,
                matches!(import.transport, Transport::Grpc { .. }),
            )?;
            ensure!(
                bindings.insert((import.capability.path.clone(), import.tenant_id.clone())),
                "同一租户的远端能力路径重复"
            );
            if let Some(route) = routes.get(&import.capability.path) {
                ensure!(
                    route.capability == import.capability,
                    "不同租户的同一路径必须具有相同契约"
                );
            } else {
                let input = validator(&import.capability.input_schema)?;
                let output = validator(&import.capability.output_schema)?;
                routes.insert(
                    import.capability.path.clone(),
                    Route {
                        capability: import.capability.clone(),
                        input,
                        output,
                        tenants: BTreeMap::new(),
                    },
                );
            }
            pending.push((import, auth));
        }
        for (import, auth) in pending {
            let binding = match import.transport {
                Transport::Mcp { tool, output } => Binding::Mcp(
                    Mcp::open(&import.endpoint, auth, tool, output, &import.capability).await?,
                ),
                Transport::Grpc {
                    descriptor_set,
                    service,
                    method,
                } => {
                    let descriptor = path.parent().unwrap_or(Path::new(".")).join(descriptor_set);
                    Binding::Grpc(
                        Grpc::open(
                            &import.endpoint,
                            auth,
                            &configuration::read(&descriptor).await?,
                            &service,
                            &method,
                        )
                        .await?,
                    )
                }
            };
            let route = routes
                .get_mut(&import.capability.path)
                .context("导入契约缺失")?;
            route.tenants.insert(import.tenant_id, binding);
        }
        Ok(Self {
            routes,
            stopped: AtomicBool::new(false),
            tasks,
        })
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

pub(super) fn validator(schema: &schemars::Schema) -> Result<jsonschema::Validator> {
    // 显式禁用外部解析，即使其他 Cargo feature 启用网络也不能从 schema 读取远端或文件。
    jsonschema::options()
        .offline()
        .build(schema.as_value())
        .map_err(|error| anyhow::anyhow!("远端能力 schema 无效: {error}"))
}

#[async_trait::async_trait]
impl CapabilityProvider for RemoteCapabilities {
    fn available(&self, path: &str, tenant: &str) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self
                .routes
                .get(path)
                .is_some_and(|route| route.tenants.contains_key(tenant))
    }

    fn capabilities(&self) -> Vec<RuntimeCapability> {
        self.routes
            .values()
            .map(|route| route.capability.clone())
            .collect()
    }

    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value> {
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "远端能力提供者已停止"
        );
        ensure!(Instant::now() < request.deadline(), "远端能力调用已过期");
        let route = self
            .routes
            .get(&request.path)
            .context("远端能力路径不存在")?;
        let tenant = request
            .scope()
            .context
            .tenant_id
            .as_deref()
            .context("远端能力缺少租户")?;
        let binding = route
            .tenants
            .get(tenant)
            .context("当前租户未绑定此远端能力")?;
        ensure!(
            route.input.is_valid(&request.input),
            "远端能力输入不符合 schema"
        );
        let operation = async {
            match binding {
                Binding::Mcp(client) => {
                    client
                        .invoke(&request.input, &self.tasks, request.deadline())
                        .await
                }
                Binding::Grpc(client) => client.invoke(&request.input, request.deadline()).await,
            }
        };
        let output = tokio::time::timeout_at(request.deadline().into(), operation)
            .await
            .context("远端能力等待超时")??;
        ensure!(
            serde_json::to_vec(&output)?.len() <= az_plugin_enhancement::MAX_VALUE_BYTES,
            "远端能力输出超过配额"
        );
        ensure!(route.output.is_valid(&output), "远端能力输出不符合 schema");
        Ok(output)
    }
}
