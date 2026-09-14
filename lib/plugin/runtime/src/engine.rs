use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use az_plugin_contract::InvocationScope;
use wasmtime::{
    Collector, Config, Engine, Store,
    component::{Component, HasSelf, Linker},
};

use crate::{
    InvocationResources,
    bindings::{
        Plugin,
        aio::plugin::transport::{Phase, Request, Response},
    },
    state::InvocationState,
};

const FUEL: u64 = 50_000_000;

pub struct ComponentEngine {
    engine: Engine,
}

#[derive(Clone)]
pub struct CompiledComponent {
    component: Component,
}

pub struct ComponentInstance {
    execution: Option<Execution>,
}

struct Execution {
    store: Store<InvocationState>,
    plugin: Plugin,
}

impl ComponentEngine {
    pub fn new() -> Result<Self> {
        Self::configured(None)
    }

    pub fn with_cache(directory: &std::path::Path) -> Result<Self> {
        Self::configured(Some(crate::compilation_cache(directory)?))
    }

    fn configured(cache: Option<wasmtime::Cache>) -> Result<Self> {
        let mut config = Config::new();
        config
            .cache(cache)
            .wasm_component_model(true)
            .wasm_gc(true)
            .wasm_function_references(true)
            .collector(Collector::DeferredReferenceCounting)
            .consume_fuel(true);
        Ok(Self {
            engine: Engine::new(&config)
                .map_err(anyhow::Error::from)
                .context("创建插件引擎失败")?,
        })
    }

    pub async fn compile(&self, bytes: &[u8]) -> Result<CompiledComponent> {
        ensure!(
            bytes.len() <= 64 * 1024 * 1024,
            "Component 超过 64 MiB 配额"
        );
        let engine = self.engine.clone();
        let bytes = bytes.to_vec();
        let component = tokio::task::spawn_blocking(move || Component::new(&engine, bytes))
            .await
            .context("等待 Component 编译失败")??;
        for (name, _) in component.component_type().imports(&self.engine) {
            ensure!(
                crate::imports::permitted(name),
                "Component 导入未授权: {name}"
            );
        }
        Ok(CompiledComponent { component })
    }

    pub async fn instantiate(
        &self,
        compiled: &CompiledComponent,
        scope: InvocationScope,
        resources: InvocationResources,
    ) -> Result<ComponentInstance> {
        if let Some(storage) = &resources.storage {
            ensure!(
                storage.matches(
                    &scope.source_id,
                    scope.context.tenant_id.as_deref().unwrap_or("")
                ),
                "对象存储绑定不属于当前插件和租户"
            );
        }
        if let Some(database) = &resources.database {
            ensure!(
                database.matches(
                    &scope.source_id,
                    scope.context.tenant_id.as_deref().unwrap_or("")
                ),
                "数据库绑定不属于当前插件和租户"
            );
        }
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut InvocationState| state)?;
        let mut store = Store::new(&self.engine, InvocationState::new(scope, resources));
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL)?;
        store.fuel_async_yield_interval(Some(10_000))?;
        let plugin = tokio::time::timeout(
            Duration::from_secs(10),
            Plugin::instantiate_async(&mut store, &compiled.component, &linker),
        )
        .await
        .context("实例化 Component 超时")??;
        Ok(ComponentInstance {
            execution: Some(Execution { store, plugin }),
        })
    }
}

impl ComponentInstance {
    pub(crate) fn rebind_revision(&mut self, revision: &str) -> Result<()> {
        self.execution
            .as_mut()
            .context("实例已停止")?
            .store
            .data_mut()
            .scope
            .revision = revision.into();
        Ok(())
    }

    fn begin(&mut self, executing: bool) -> Result<Execution> {
        // 调用期间转移所有权，future 被取消时 Store 和未提交事务随之销毁。
        let mut execution = self.execution.take().context("实例已失效，必须重新创建")?;
        execution.store.set_fuel(FUEL)?;
        execution.store.data_mut().executing = executing;
        execution.store.data_mut().log_events = 0;
        Ok(execution)
    }
    pub async fn describe(
        &mut self,
    ) -> Result<crate::bindings::aio::plugin::metadata::Description> {
        let mut execution = self.begin(false)?;
        let definition = tokio::time::timeout(
            Duration::from_secs(5),
            execution.plugin.call_describe(&mut execution.store),
        )
        .await
        .context("插件描述超时")??;
        crate::metadata::validate(&definition)?;
        self.execution = Some(execution);
        Ok(definition)
    }

    pub async fn health(&mut self) -> Result<()> {
        let mut execution = self.begin(false)?;
        tokio::time::timeout(
            Duration::from_secs(5),
            execution.plugin.call_health(&mut execution.store),
        )
        .await
        .context("插件健康检查超时")??
        .map_err(|error| anyhow!("插件健康检查失败: {error}"))?;
        self.execution = Some(execution);
        Ok(())
    }

    pub async fn lifecycle(&mut self, phase: Phase) -> Result<()> {
        let mut execution = self.begin(true)?;
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            execution.plugin.call_lifecycle(&mut execution.store, phase),
        )
        .await;
        execution.store.data_mut().cleanup().await?;
        result
            .context("插件生命周期超时")??
            .map_err(|error| anyhow!("插件生命周期失败: {error}"))?;
        self.execution = Some(execution);
        Ok(())
    }

    pub async fn handle(
        &mut self,
        request: Request,
        context: az_plugin_contract::RequestContext,
    ) -> Result<Response> {
        ensure!(request.body.len() <= 32 * 1024 * 1024, "请求体超过配额");
        ensure!(
            context.tenant_id
                == self
                    .execution
                    .as_ref()
                    .context("实例已失效，必须重新创建")?
                    .store
                    .data()
                    .scope
                    .context
                    .tenant_id,
            "实例不能跨租户复用"
        );
        let mut execution = self.begin(true)?;
        execution.store.data_mut().scope.context = context;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            execution.plugin.call_handle(&mut execution.store, &request),
        )
        .await;
        execution.store.data_mut().cleanup().await?;
        let response = result.context("插件请求超时")??;
        ensure!((100..=599).contains(&response.status), "响应状态码无效");
        ensure!(
            response.body.len() <= 32 * 1024 * 1024 && response.headers.len() <= 64,
            "响应超过配额"
        );
        self.execution = Some(execution);
        Ok(response)
    }
}
