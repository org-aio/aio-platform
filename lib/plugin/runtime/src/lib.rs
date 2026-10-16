#![forbid(unsafe_code)]

#[cfg(feature = "wasm")]
mod compilation;
#[cfg(feature = "wasm")]
pub use compilation::compilation_cache;

#[cfg(all(test, feature = "wasm"))]
mod cancellation_tests;
mod component_database;
mod cryptography;
mod database;
#[cfg(feature = "wasm")]
mod database_parameters;
mod database_policy;
#[cfg(feature = "wasm")]
mod database_query;
#[cfg(feature = "wasm")]
mod engine;
mod file_storage;
#[cfg(feature = "wasm")]
mod imports;
#[cfg(feature = "wasm")]
mod metadata;
mod persistence;
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
mod wasm_transport;

pub use component_database::ComponentDatabase;
pub use cryptography::Keyring;
pub use database::ScopedDatabase;
#[cfg(feature = "wasm")]
pub use engine::{CompiledComponent, ComponentEngine, ComponentInstance};
pub use file_storage::FileObjectStorage;
pub use provision::DatabaseProvisioner;
#[cfg(feature = "wasm")]
pub use registry::{PersistentComponentSlot, StoredRelease};
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
