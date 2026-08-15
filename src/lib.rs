#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod application_compiler;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod business_module;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod compiled_artifact;
mod compiler;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod convention_endpoint;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod convention_source;
mod definition;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod form_state;
#[cfg(any(feature = "runtime-ui", test))]
mod identifier_generation;
mod image;
#[cfg(any(feature = "runtime-ui", test))]
mod model_audit;
#[cfg(any(feature = "runtime-ui", test))]
mod page_endpoint_draft;
#[cfg(any(feature = "runtime-ui", test))]
mod page_renderer_draft;
#[cfg(feature = "runtime-ui")]
mod page_runtime;
mod patch;
#[cfg(feature = "runtime-ui")]
mod runtime_bridge;
#[cfg(any(feature = "runtime-ui", test))]
mod runtime_record_form;
#[cfg(feature = "runtime-ui")]
mod runtime_tree;
mod studio_contract;
#[cfg(any(feature = "runtime-ui", test))]
mod studio_navigation;
mod vm;

#[cfg(feature = "runtime-ui")]
pub mod browser_http;
#[cfg(feature = "runtime-ui")]
mod ui;
#[cfg(feature = "runtime-ui")]
pub use ui::StudioPage;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod capability;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod patch_agent;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod program_runtime;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod program_store;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod studio_http;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use application_compiler::*;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use business_module::*;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use compiled_artifact::*;
pub use compiler::{
    CompileFailure, CompilerStage, Diagnostic, DiagnosticSeverity, ProgramCompiler, compile_page,
    content_hash, convention_page_module_name, convention_page_path,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use convention_endpoint::*;
pub use definition::*;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use form_state::FormStateExtractor;
pub use image::*;
#[cfg(feature = "runtime-ui")]
pub use page_runtime::*;
pub use patch::*;
#[cfg(feature = "runtime-ui")]
pub use runtime_bridge::*;
pub use studio_contract::*;
pub use vm::*;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use patch_agent::ProgramPatchAgent;
