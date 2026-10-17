#![forbid(unsafe_code)]

#[cfg(feature = "wasm")]
mod compilation;
#[cfg(feature = "wasm")]
pub use compilation::compilation_cache;

#[cfg(all(test, feature = "wasm", feature = "postgres"))]
mod cancellation_tests;
mod component_database;
mod component_releases;
mod cryptography;
mod database;
mod database_backend;
#[cfg(feature = "postgres")]
mod database_parameters;
#[cfg(feature = "postgres")]
mod database_policy;
#[cfg(feature = "postgres")]
mod database_query;
mod database_values;
#[cfg(feature = "wasm")]
mod engine;
mod file_storage;
#[cfg(feature = "wasm")]
mod imports;
#[cfg(feature = "wasm")]
mod metadata;
#[cfg(feature = "postgres")]
mod persistence;
#[cfg(feature = "postgres")]
mod postgres_database;
#[cfg(feature = "postgres")]
mod postgres_releases;
#[cfg(feature = "postgres")]
mod provision;
#[cfg(feature = "wasm")]
mod registry;
#[cfg(feature = "wasm")]
mod release;
mod resources;
mod services;
#[cfg(feature = "wasm")]
mod state;
mod storage;
#[cfg(all(test, feature = "wasm"))]
mod transaction_tests;
#[cfg(feature = "wasm")]
mod wasm_database;
#[cfg(feature = "wasm")]
mod wasm_transport;

pub use component_database::ComponentDatabase;
pub use component_releases::{ComponentReleases, ReleaseLease, ReleaseVersion, StoredRelease};
pub use cryptography::Keyring;
pub use database::{ScopedDatabase, ScopedTransaction};
pub use database_backend::{
    DatabaseBackend, DatabaseEndpoint, DatabaseTransaction, ProcessDatabaseConnection,
};
pub use database_values::{DatabaseRows, DatabaseValue};
#[cfg(feature = "wasm")]
pub use engine::{CompiledComponent, ComponentEngine, ComponentInstance};
pub use file_storage::FileObjectStorage;
#[cfg(feature = "postgres")]
pub use postgres_database::PostgresDatabase;
#[cfg(feature = "postgres")]
pub use postgres_releases::PostgresReleases;
#[cfg(feature = "postgres")]
pub use provision::DatabaseProvisioner;
#[cfg(feature = "wasm")]
pub use registry::PersistentComponentSlot;
#[cfg(feature = "wasm")]
pub use release::{ComponentSlot, ReleaseSnapshot};
pub use resources::InvocationResources;
pub use services::HostServices;
pub use storage::{ObjectKey, ObjectStorage, ObjectStore};

pub const FRONTEND_HOST: &str = include_str!("../../../../sdk/web/host.mjs");
pub const FRONTEND_GUEST: &str = include_str!("../../../../sdk/web/guest.js");
pub const FRONTEND_WASM: &str = include_str!("../../../../sdk/web/wasm.js");
pub const FRONTEND_LIFECYCLE: &str = include_str!("../../../../sdk/web/lifecycle.js");
pub const FRONTEND_NAVIGATION: &str = include_str!("../../../../sdk/web/navigation.js");

#[cfg(feature = "wasm")]
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "../contract/wit",
        world: "plugin",
        imports: { default: async | trappable },
        exports: { default: async },
    });
}
