use super::{
    RuntimeState, frontend_access::FrontendAccess, process::ProcessManager,
    repository::RepositoryInstaller, store::PluginStore, wasm::WasmManager,
};
use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

impl RuntimeState {
    pub async fn isolated_admin_test(
        identity: Arc<dyn crate::identity::IdentityProvider>,
        database: &str,
        origin: &str,
        cache: &Path,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database)
            .await?;
        let store = Arc::new(PluginStore::new(pool));
        store.migrate().await?;
        let workers = dill::Catalog::builder()
            .add_value(store.pool.clone())
            .add::<crate::generated::worker::WorkerServiceImpl>()
            .build()
            .get_one::<dyn crate::generated::worker::WorkerService>()?;
        Ok(Self {
            workers,
            config: Arc::new(crate::configuration::HostConfig {
                database_url: database.into(),
                cache_root: cache.into(),
                public_origin: origin.into(),
                component_storage: None,
                default_plugins: vec![],
                delivery: None,
                development: None,
            }),
            development: Arc::default(),
            store,
            repository: Arc::new(RepositoryInstaller::new(cache.to_path_buf())),
            identity,
            activation_locks: Arc::new(Mutex::new(HashMap::new())),
            publication_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            frontend: Arc::new(FrontendAccess::new(origin)?),
            process: Arc::new(ProcessManager::new()?),
            wasm: Arc::new(WasmManager::new()?),
            components: None,
        })
    }
}
