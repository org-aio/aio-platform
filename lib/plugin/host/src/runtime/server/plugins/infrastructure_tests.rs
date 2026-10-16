use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use az_plugin_core::PluginFuture;
use az_plugin_runtime::Keyring;
use sqlx::PgPool;

use super::*;

#[derive(Clone)]
struct Providers {
    pool: PgPool,
    keyring: Keyring,
    calls: Arc<AtomicUsize>,
}

impl Providers {
    fn lazy() -> Result<Self> {
        Ok(Self {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgresql://test@127.0.0.1/infrastructure_test")?,
            keyring: Keyring::new(
                "injected".into(),
                BTreeMap::from([("injected".into(), [7; 32])]),
            )?,
            calls: Arc::default(),
        })
    }
}

#[async_trait::async_trait]
impl DatabaseProvider for Providers {
    async fn connect(&self, _: &crate::configuration::HostConfig) -> Result<PgPool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.pool.clone())
    }
}

#[async_trait::async_trait]
impl KeyringProvider for Providers {
    async fn load(&self, _: &crate::configuration::HostConfig) -> Result<Keyring> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.keyring.clone())
    }
}

impl Plugin<RuntimeAssembly> for Providers {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(self.clone())
                .bind::<dyn DatabaseProvider, Self>()
                .bind::<dyn KeyringProvider, Self>();
            Ok(())
        })
    }
}

#[dill::component]
#[dill::scope(dill::Singleton)]
struct Injected {
    pool: Arc<PgPool>,
    keyring: Arc<Keyring>,
}

#[derive(Clone, Copy)]
enum Failure {
    None,
    Initialize,
    Resolve,
    Panic,
}

struct Consumer {
    events: Arc<Mutex<Vec<&'static str>>>,
    failure: Failure,
}

impl Plugin<RuntimeAssembly> for Consumer {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<DatabasePlugin>(),
            TypeId::of::<KeyringPlugin>(),
        ]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<Injected>();
            assembly
                .services
                .add_value(self.events.clone())
                .add_value(self.failure);
            assembly.require_service::<Injected>();
            match self.failure {
                Failure::Resolve => {
                    assembly.declare_resource::<u64>()?;
                    assembly.require_service::<u64>();
                }
                Failure::Panic => {
                    assembly
                        .services
                        .add_value_lazy(|| -> u64 { panic!("测试服务构造失败") });
                    assembly.require_service::<u64>();
                }
                _ => {}
            }
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let injected = state.service::<Injected>()?;
                    assert!(!injected.pool.is_closed());
                    assert!(Arc::ptr_eq(&injected.keyring, &state.service::<Keyring>()?));
                    record(&state, "initialize")?;
                    anyhow::ensure!(
                        !matches!(*state.service::<Failure>()?, Failure::Initialize),
                        "测试初始化失败"
                    );
                    Ok(())
                })
            });
            assembly.on_start::<Self>(|state, _| Box::pin(async move { record(&state, "start") }));
            assembly.on_ready::<Self>(|state, _| Box::pin(async move { record(&state, "ready") }));
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    assert!(!state.service::<PgPool>()?.is_closed());
                    record(&state, "stop")
                })
            });
            Ok(())
        })
    }
}

fn record(state: &RuntimeState, event: &'static str) -> Result<()> {
    state
        .service::<Arc<Mutex<Vec<&'static str>>>>()?
        .lock()
        .map_err(|_| anyhow::anyhow!("测试事件锁损坏"))?
        .push(event);
    Ok(())
}

fn plugins(
    providers: Providers,
    events: Arc<Mutex<Vec<&'static str>>>,
    failure: Failure,
) -> Vec<DynPlugin<RuntimeAssembly>> {
    // 依赖方故意在前，验证初始化顺序来自插件依赖图。
    vec![
        Arc::new(Consumer { events, failure }),
        Arc::new(KeyringPlugin),
        Arc::new(DatabasePlugin),
        Arc::new(providers),
    ]
}

#[tokio::test]
async fn injected_resources_are_shared_and_initialize_without_business_restoration() -> Result<()> {
    for restore in [false, true] {
        let providers = Providers::lazy()?;
        let root = tempfile::tempdir()?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let state = RuntimeState::initialize_selected(
            tests::config(root.path()),
            plugins(providers.clone(), events.clone(), Failure::None),
            restore,
        )
        .await?;
        assert_eq!(providers.calls.load(Ordering::SeqCst), 2);
        assert!(!root.path().join("cache").exists());
        let injected = state.service::<Injected>()?;
        assert!(Arc::ptr_eq(&injected, &state.service::<Injected>()?));
        assert!(Arc::ptr_eq(&injected.pool, &state.service::<PgPool>()?));
        state.shutdown().await?;
        state.shutdown().await?;
        assert!(providers.pool.is_closed());
        let weak = Arc::downgrade(&injected);
        drop(injected);
        drop(state);
        assert!(weak.upgrade().is_none());
        let actual = events
            .lock()
            .map_err(|_| anyhow::anyhow!("测试事件锁损坏"))?
            .clone();
        assert_eq!(
            actual,
            if restore {
                vec!["initialize", "start", "ready", "stop"]
            } else {
                vec!["initialize", "stop"]
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn initialization_and_resolution_failures_close_owned_resources_before_returning()
-> Result<()> {
    for failure in [Failure::Initialize, Failure::Resolve, Failure::Panic] {
        let providers = Providers::lazy()?;
        let root = tempfile::tempdir()?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let result = RuntimeState::initialize(
            tests::config(root.path()),
            plugins(providers.clone(), events.clone(), failure),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(providers.calls.load(Ordering::SeqCst), 2);
        assert!(providers.pool.is_closed());
        assert_eq!(
            *events
                .lock()
                .map_err(|_| anyhow::anyhow!("测试事件锁损坏"))?,
            ["initialize", "stop"]
        );
    }
    Ok(())
}

#[tokio::test]
async fn missing_or_ambiguous_providers_fail_before_resource_initialization() -> Result<()> {
    assert!(
        tests::extensions(vec![Arc::new(DatabasePlugin)])
            .await
            .is_err()
    );
    let providers = Providers::lazy()?;
    assert!(
        tests::extensions(vec![
            Arc::new(providers.clone()),
            Arc::new(FileKeyringPlugin),
            Arc::new(KeyringPlugin)
        ])
        .await
        .is_err()
    );
    assert_eq!(providers.calls.load(Ordering::SeqCst), 0);
    providers.pool.close().await;
    Ok(())
}

#[test]
fn resources_reject_duplicate_declarations_and_values() -> Result<()> {
    let mut services = CatalogBuilder::new();
    resource::declare::<u64>(&mut services)?;
    assert!(resource::declare::<u64>(&mut services).is_err());
    let catalog = services.build();
    assert!(catalog.get_one::<u64>().is_err());
    let slot = catalog.get_one::<Resource<u64>>()?;
    slot.provide(Arc::new(42))?;
    assert!(slot.provide(Arc::new(99)).is_err());
    assert_eq!(*catalog.get_one::<u64>()?, 42);
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn custom_database_provider_runs_real_storage_and_closes_on_shutdown() -> Result<()> {
    let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    anyhow::ensure!(
        database.contains("127.0.0.1") && database.contains("_test"),
        "只接受本机独立测试库"
    );
    let mut providers = Providers::lazy()?;
    providers.pool.close().await;
    providers.pool = PgPool::connect(&database).await?;
    let root = tempfile::tempdir()?;
    // 配置里的连接地址无效；必须真正使用应用注入的 Provider 才能完成迁移与查询。
    let state = RuntimeState::initialize(
        tests::config(root.path()),
        vec![
            Arc::new(StorePlugin),
            Arc::new(DatabasePlugin),
            Arc::new(providers.clone()),
        ],
    )
    .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plugin_sources")
        .fetch_one(state.service::<PgPool>()?.as_ref())
        .await?;
    assert!(count >= 0);
    assert!(state.service::<Keyring>().is_err());
    assert!(state.wasm().is_err());
    state.shutdown().await?;
    assert!(providers.pool.is_closed());
    Ok(())
}
