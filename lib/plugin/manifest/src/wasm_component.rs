use anyhow::{Context as _, Result, anyhow, bail, ensure};
use wasmtime::component::{Component, Linker};
use wasmtime::{Collector, Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wit_component::{DecodedWasm, decode};
use wit_parser::{Function, FunctionKind, Type, WorldItem, WorldKey};

use crate::{PageDefinition, validate_page_definitions};

// Component 适配层在 definition 阶段会初始化序列化和页面模型，跨语言产物通常比业务调用消耗更多 fuel。
const FUEL_PER_DEFINITION: u64 = 25_000_000;
const MAX_DEFINITION_BYTES: usize = 1024 * 1024;
const MAX_MEMORY_BYTES: usize = 128 * 1024 * 1024;
const MAX_TABLE_ELEMENTS: usize = 100_000;

struct StoreState {
    limits: StoreLimits,
}

pub fn validate_wasm_component(bytes: &[u8]) -> Result<Vec<PageDefinition>> {
    validate_component_abi(bytes)?;
    let engine = component_engine()?;
    let component = Component::new(&engine, bytes)
        .map_err(|error| anyhow!("编译 Wasm Component 失败: {error:#}"))?;
    let linker = Linker::new(&engine);
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_MEMORY_BYTES)
        .table_elements(MAX_TABLE_ELEMENTS)
        .instances(128)
        .tables(32)
        .memories(8)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(&engine, StoreState { limits });
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(FUEL_PER_DEFINITION)
        .map_err(|error| anyhow!("设置 Wasm 校验 fuel 失败: {error:#}"))?;
    let instance = linker
        .instantiate(&mut store, &component)
        .map_err(|error| anyhow!("实例化 Wasm Component 失败: {error:#}"))?;
    let definition = instance
        .get_typed_func::<(), (String,)>(&mut store, "definition")
        .map_err(|error| anyhow!("Wasm Component 缺少 definition 导出: {error:#}"))?;
    let (definition,) = definition
        .call(&mut store, ())
        .map_err(|error| anyhow!("调用 Wasm Component definition 失败: {error:#}"))?;
    ensure!(
        definition.len() <= MAX_DEFINITION_BYTES,
        "Wasm Component PageDefinition 不能超过 {MAX_DEFINITION_BYTES} 字节"
    );
    let pages = crate::parse_page_definitions(definition.as_bytes())
        .context("解析 Wasm Component PageDefinition 失败")?;
    ensure!(!pages.is_empty(), "Wasm Component 至少需要贡献一个页面");
    validate_page_definitions(&pages)?;
    Ok(pages)
}

fn component_engine() -> Result<Engine> {
    let mut config = Config::new();
    config
        .wasm_component_model(true)
        .wasm_function_references(true)
        .wasm_gc(true)
        .collector(Collector::DeferredReferenceCounting)
        .consume_fuel(true);
    Engine::new(&config).map_err(|error| anyhow!("创建 Wasm Component 引擎失败: {error:#}"))
}

fn validate_component_abi(bytes: &[u8]) -> Result<()> {
    let decoded = decode(bytes).context("解析 Wasm Component WIT 失败")?;
    let (resolve, world_id) = match decoded {
        DecodedWasm::Component(resolve, world_id) => (resolve, world_id),
        DecodedWasm::WitPackage(_, _) => bail!("artifact 是 WIT package，不是 Wasm Component"),
    };
    let world = &resolve.worlds[world_id];
    ensure!(
        world.imports.is_empty(),
        "Wasm Component 包含未授权导入: {}",
        world.imports.len()
    );
    ensure!(
        world.exports.len() == 2,
        "Wasm Component 必须仅导出 definition 和 handle"
    );
    let definition = exported_function(
        world.exports.get(&WorldKey::Name("definition".to_owned())),
        "definition",
    )?;
    ensure!(definition.params.is_empty(), "definition 不能声明参数");
    ensure!(
        definition.result.as_ref() == Some(&Type::String),
        "definition 必须返回 string"
    );
    let handle = exported_function(
        world.exports.get(&WorldKey::Name("handle".to_owned())),
        "handle",
    )?;
    ensure!(
        handle.params.len() == 1 && handle.params[0].1 == Type::String,
        "handle 必须接受一个 string 参数"
    );
    ensure!(
        handle.result.as_ref() == Some(&Type::String),
        "handle 必须返回 string"
    );
    Ok(())
}

fn exported_function<'a>(item: Option<&'a WorldItem>, name: &str) -> Result<&'a Function> {
    let Some(WorldItem::Function(function)) = item else {
        bail!("Wasm Component 缺少 {name} 函数导出");
    };
    ensure!(
        function.kind == FunctionKind::Freestanding,
        "Wasm Component {name} 必须是同步独立函数"
    );
    Ok(function)
}
