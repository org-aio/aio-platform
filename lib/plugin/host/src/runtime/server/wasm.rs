use std::{
    collections::{HashMap, hash_map::Entry},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::{Context as _, Result, anyhow};
use wasmtime::component::{Component, Instance, Linker};
use wasmtime::{Collector, Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

#[cfg(test)]
use crate::runtime::PluginRequest;
use crate::runtime::{ComponentResponse, PageDefinition};

const FUEL_PER_DEFINITION: u64 = 25_000_000;
const FUEL_PER_HANDLER: u64 = 20_000_000;
const MAX_CACHED_COMPONENTS: usize = 16;
const MAX_WASM_MEMORY_BYTES: usize = 128 * 1024 * 1024;
const MAX_WASM_TABLE_ELEMENTS: usize = 100_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct InstanceKey {
    tenant_id: String,
    source_id: String,
    revision: String,
}

impl InstanceKey {
    fn new(tenant_id: &str, source_id: &str, revision: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_owned(),
            source_id: source_id.to_owned(),
            revision: revision.to_owned(),
        }
    }
}

struct StoreState {
    limits: StoreLimits,
}

struct TenantInstance {
    store: Store<StoreState>,
    instance: Instance,
}

pub struct WasmActivation {
    pub pages: Vec<PageDefinition>,
    pub created: bool,
}

pub struct WasmManager {
    engine: Engine,
    components: Mutex<HashMap<PathBuf, Arc<Component>>>,
    instances: Mutex<HashMap<InstanceKey, Arc<Mutex<TenantInstance>>>>,
}

impl WasmManager {
    pub fn new() -> Result<Self> {
        Self::with_cache(None)
    }

    pub(super) fn with_cache(directory: Option<&Path>) -> Result<Self> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .wasm_function_references(true)
            .wasm_gc(true)
            .collector(Collector::DeferredReferenceCounting)
            .consume_fuel(true);
        if let Some(directory) = directory {
            config.cache(Some(az_plugin_runtime::compilation_cache(directory)?));
        }
        let engine = Engine::new(&config)
            .map_err(|error| anyhow!("创建 Wasm Component 引擎失败: {error:#}"))?;
        Ok(Self {
            engine,
            components: Mutex::new(HashMap::new()),
            instances: Mutex::new(HashMap::new()),
        })
    }

    pub fn activate(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        artifact: &Path,
    ) -> Result<WasmActivation> {
        let key = InstanceKey::new(tenant_id, source_id, revision);
        if let Some(instance) = self.instances()?.get(&key).cloned() {
            return Ok(WasmActivation {
                pages: call_definition(&instance)?,
                created: false,
            });
        }
        let candidate = Arc::new(Mutex::new(self.instantiate(artifact)?));
        let (instance, created) = {
            let mut instances = self.instances()?;
            match instances.entry(key.clone()) {
                Entry::Occupied(entry) => (entry.get().clone(), false),
                Entry::Vacant(entry) => (entry.insert(candidate).clone(), true),
            }
        };
        match call_definition(&instance) {
            Ok(pages) => Ok(WasmActivation { pages, created }),
            Err(error) => {
                if created {
                    self.instances()?.remove(&key);
                }
                Err(error)
            }
        }
    }

    pub fn deactivate(&self, tenant_id: &str, source_id: &str, revision: &str) -> Result<bool> {
        let key = InstanceKey::new(tenant_id, source_id, revision);
        Ok(self.instances()?.remove(&key).is_some())
    }

    pub(super) fn bind_revision(
        &self,
        tenant: &str,
        source: &str,
        backend: &str,
        revision: &str,
    ) -> Result<()> {
        let mut instances = self.instances()?;
        let instance = instances
            .get(&InstanceKey::new(tenant, source, backend))
            .cloned()
            .context("开发后端实例不存在")?;
        instances.insert(InstanceKey::new(tenant, source, revision), instance);
        Ok(())
    }

    pub(super) fn retain_development_revisions(
        &self,
        tenant: &str,
        source: &str,
        backend: &str,
        revision: &str,
    ) -> Result<()> {
        self.instances()?.retain(|key, _| {
            key.tenant_id != tenant
                || key.source_id != source
                || key.revision == backend
                || key.revision == revision
        });
        Ok(())
    }

    pub fn handle(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
        request: String,
    ) -> Result<ComponentResponse> {
        let key = InstanceKey::new(tenant_id, source_id, revision);
        let instance = self
            .instances()?
            .get(&key)
            .cloned()
            .context("当前租户的 Wasm Component 实例未激活")?;
        call_handler(&instance, request)
    }

    fn instantiate(&self, artifact: &Path) -> Result<TenantInstance> {
        let component = self.component(artifact)?;
        let linker = Linker::new(&self.engine);
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_WASM_MEMORY_BYTES)
            .table_elements(MAX_WASM_TABLE_ELEMENTS)
            .instances(128)
            .tables(32)
            .memories(8)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, StoreState { limits });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(FUEL_PER_HANDLER)
            .map_err(|error| anyhow!("设置 Wasm fuel 失败: {error:#}"))?;
        let instance = linker
            .instantiate(&mut store, component.as_ref())
            .map_err(|error| anyhow!("实例化 Wasm Component 失败: {error:#}"))?;
        Ok(TenantInstance { store, instance })
    }

    fn component(&self, artifact: &Path) -> Result<Arc<Component>> {
        if let Some(component) = self.components()?.get(artifact).cloned() {
            return Ok(component);
        }
        let component = Arc::new(Component::from_file(&self.engine, artifact).map_err(
            |error| anyhow!("编译 Wasm Component 失败 {}: {error:#}", artifact.display()),
        )?);
        let mut components = self.components()?;
        if let Some(cached) = components.get(artifact) {
            return Ok(cached.clone());
        }
        if components.len() >= MAX_CACHED_COMPONENTS
            && let Some(stale) = components.keys().next().cloned()
        {
            components.remove(&stale);
        }
        Ok(components
            .entry(artifact.to_path_buf())
            .or_insert_with(|| component)
            .clone())
    }

    fn components(&self) -> Result<MutexGuard<'_, HashMap<PathBuf, Arc<Component>>>> {
        self.components
            .lock()
            .map_err(|_| anyhow!("Wasm Component 缓存锁已损坏"))
    }

    fn instances(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<InstanceKey, Arc<Mutex<TenantInstance>>>>> {
        self.instances
            .lock()
            .map_err(|_| anyhow!("Wasm Component 实例锁已损坏"))
    }

    #[cfg(test)]
    fn active_instances(&self) -> Result<usize> {
        Ok(self.instances()?.len())
    }
}

fn call_definition(instance: &Arc<Mutex<TenantInstance>>) -> Result<Vec<PageDefinition>> {
    let mut instance = instance
        .lock()
        .map_err(|_| anyhow!("Wasm 租户实例锁已损坏"))?;
    instance
        .store
        .set_fuel(FUEL_PER_DEFINITION)
        .map_err(|error| anyhow!("重置 Wasm fuel 失败: {error:#}"))?;
    let component = instance.instance;
    let handle = component
        .get_typed_func::<(), (String,)>(&mut instance.store, "definition")
        .map_err(|error| anyhow!("Wasm Component 缺少 definition 导出: {error:#}"))?;
    let (json,) = handle
        .call(&mut instance.store, ())
        .map_err(|error| anyhow!("调用 Wasm Component definition 失败: {error:#}"))?;
    az_plugin_manifest::parse_page_definitions(json.as_bytes())
        .map_err(|error| anyhow!("解析 Wasm Component PageDefinition 失败: {error}"))
}

fn call_handler(
    instance: &Arc<Mutex<TenantInstance>>,
    request: String,
) -> Result<ComponentResponse> {
    let mut instance = instance
        .lock()
        .map_err(|_| anyhow!("Wasm 租户实例锁已损坏"))?;
    instance
        .store
        .set_fuel(FUEL_PER_HANDLER)
        .map_err(|error| anyhow!("重置 Wasm fuel 失败: {error:#}"))?;
    let component = instance.instance;
    let handle = component
        .get_typed_func::<(String,), (String,)>(&mut instance.store, "handle")
        .map_err(|error| anyhow!("Wasm Component 缺少 handle 导出: {error:#}"))?;
    let (json,) = handle
        .call(&mut instance.store, (request,))
        .map_err(|error| anyhow!("调用 Wasm Component handle 失败: {error:#}"))?;
    serde_json::from_str(&json).map_err(|error| anyhow!("解析 Wasm Component 响应失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_instance_keys_by_tenant_source_and_revision() {
        let base = InstanceKey::new("tenant-a", "source-a", &"a".repeat(40));
        assert_eq!(
            base,
            InstanceKey::new("tenant-a", "source-a", &"a".repeat(40))
        );
        assert_ne!(
            base,
            InstanceKey::new("tenant-b", "source-a", &"a".repeat(40))
        );
        assert_ne!(
            base,
            InstanceKey::new("tenant-a", "source-b", &"a".repeat(40))
        );
        assert_ne!(
            base,
            InstanceKey::new("tenant-a", "source-a", &"b".repeat(40))
        );
    }

    #[test]
    fn starts_without_active_instances() -> Result<()> {
        let manager = WasmManager::new()?;
        assert_eq!(manager.active_instances()?, 0);
        Ok(())
    }

    #[test]
    #[ignore = "需要 AIO_TEST_WASM_COMPONENT 指向真实插件产物"]
    fn activates_reuses_and_destroys_tenant_instances() -> Result<()> {
        let artifact = std::env::var("AIO_TEST_WASM_COMPONENT")?;
        let manager = WasmManager::new()?;
        let revision = "a".repeat(40);
        let first = manager.activate("tenant-a", "source-a", &revision, Path::new(&artifact))?;
        assert!(first.created);
        assert!(!first.pages.is_empty());
        let action_page = first
            .pages
            .iter()
            .find_map(|page| match &page.body {
                crate::runtime::PageBody::Actions {
                    title,
                    content,
                    actions,
                    ..
                } => Some((
                    page.id.clone(),
                    title.clone(),
                    content.clone(),
                    actions.clone(),
                )),
                _ => None,
            })
            .context("测试 Component 没有 actions 页面")?;
        let action_id = action_page
            .3
            .first()
            .context("测试 Component 没有页面动作")?
            .id
            .clone();

        let reused = manager.activate("tenant-a", "source-a", &revision, Path::new(&artifact))?;
        assert!(!reused.created);
        manager.activate("tenant-b", "source-a", &revision, Path::new(&artifact))?;
        assert_eq!(manager.active_instances()?, 2);

        let response = manager.handle(
            "tenant-a",
            "source-a",
            &revision,
            serde_json::to_string(&PluginRequest::ServiceRequest {
                method: "POST".to_owned(),
                path: "/echo".to_owned(),
                query: None,
                body: "tenant scoped".to_owned(),
                tenant_id: "tenant-a".to_owned(),
                user_id: "user-a".to_owned(),
            })?,
        )?;
        assert_eq!(response.status, 200);
        assert!(response.body.contains("tenant-a"));

        let action = |tenant_id: &str, count: u64| {
            manager.handle(
                tenant_id,
                "source-a",
                &revision,
                serde_json::to_string(&PluginRequest::PageAction {
                    page_id: action_page.0.clone(),
                    action_id: action_id.clone(),
                    tenant_id: tenant_id.to_owned(),
                    user_id: "user-a".to_owned(),
                    body: crate::runtime::PageBody::Actions {
                        title: action_page.1.clone(),
                        content: action_page.2.clone(),
                        state: [("count".to_owned(), serde_json::json!(count))]
                            .into_iter()
                            .collect(),
                        actions: action_page.3.clone(),
                    },
                })?,
            )
        };
        let count = |response: ComponentResponse| -> Result<u64> {
            let result = serde_json::from_str::<crate::runtime::PageActionResult>(&response.body)?;
            let crate::runtime::PageBody::Actions { state, .. } = result.body else {
                anyhow::bail!("页面动作没有返回 actions 页面体");
            };
            state
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .context("页面动作没有返回整数 count 状态")
        };
        assert_eq!(count(action("tenant-a", 0)?)?, 1);
        assert_eq!(count(action("tenant-b", 0)?)?, 1);
        assert_eq!(count(action("tenant-a", 1)?)?, 2);

        assert!(manager.deactivate("tenant-a", "source-a", &revision)?);
        assert_eq!(manager.active_instances()?, 1);
        assert!(
            manager
                .handle("tenant-a", "source-a", &revision, "{}".to_owned())
                .is_err()
        );
        Ok(())
    }
}
