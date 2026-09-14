#![forbid(unsafe_code)]

#[cfg(feature = "validation")]
mod frontend;
mod model;
mod navigation;
#[cfg(feature = "schema")]
mod schema;
#[cfg(feature = "validation")]
mod validation;
#[cfg(feature = "validation")]
mod wasm_component;

#[cfg(feature = "validation")]
pub use frontend::{
    MAX_FRONTEND_FILES, frontend_files, validate_frontend_pages, validate_frontend_path,
};
pub use model::{
    CapabilityManifest, ComponentResponse, FrontendManifest, MarketplaceManifest,
    MenuGroupDefinition, PageActionDefinition, PageActionResult, PageBody, PageDefinition,
    PluginManifest, PluginRequest, PluginRuntime, RepositoryManifest, RepositoryPackage,
    RuntimeManifest, SceneDefinition, SubpluginManifest,
};
#[cfg(feature = "validation")]
pub use navigation::parse_page_definitions;
pub use navigation::{MenuBranch, MenuNode, MenuPage, PageDocument, SceneTree};
#[cfg(feature = "schema")]
pub use schema::{PluginSchema, schemas};
#[cfg(feature = "validation")]
pub use validation::{
    ValidationReport, artifact_path, parse_manifest, read_manifest, validate_declared_pages,
    validate_host_compatibility, validate_manifest, validate_page_definitions, validate_repository,
};
#[cfg(feature = "validation")]
pub use wasm_component::validate_wasm_component;
