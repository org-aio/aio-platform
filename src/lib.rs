#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod compiled_artifact;
mod compiler;
#[cfg(not(target_arch = "wasm32"))]
mod convention_contract;
#[cfg(not(target_arch = "wasm32"))]
mod convention_endpoint;
#[cfg(not(target_arch = "wasm32"))]
mod convention_file;
mod definition;
#[cfg(not(target_arch = "wasm32"))]
mod form_state;
#[cfg(any(target_arch = "wasm32", test))]
mod identifier_generation;
mod image;
#[cfg(any(target_arch = "wasm32", test))]
mod model_audit;
#[cfg(not(target_arch = "wasm32"))]
mod native_contract;
#[cfg(any(target_arch = "wasm32", test))]
mod page_endpoint_draft;
#[cfg(any(target_arch = "wasm32", test))]
mod page_renderer_draft;
#[cfg(target_arch = "wasm32")]
mod page_runtime;
mod patch;
#[cfg(target_arch = "wasm32")]
mod runtime_bridge;
#[cfg(any(target_arch = "wasm32", test))]
mod runtime_record_form;
#[cfg(target_arch = "wasm32")]
mod runtime_tree;
mod studio_contract;
#[cfg(any(target_arch = "wasm32", test))]
mod studio_navigation;
mod vm;

#[cfg(target_arch = "wasm32")]
pub mod browser_http;
#[cfg(target_arch = "wasm32")]
mod ui;
#[cfg(target_arch = "wasm32")]
pub use ui::StudioPage;

#[cfg(not(target_arch = "wasm32"))]
pub mod capability;
#[cfg(not(target_arch = "wasm32"))]
mod patch_agent;
#[cfg(not(target_arch = "wasm32"))]
pub mod program_runtime;
#[cfg(not(target_arch = "wasm32"))]
pub mod program_store;
#[cfg(not(target_arch = "wasm32"))]
pub mod studio_http;

#[cfg(not(target_arch = "wasm32"))]
pub use compiled_artifact::*;
pub use compiler::{
    CompileFailure, CompilerStage, Diagnostic, DiagnosticSeverity, ProgramCompiler, compile_page,
    content_hash, convention_page_module_name, convention_page_path,
};
#[cfg(not(target_arch = "wasm32"))]
pub use convention_contract::*;
#[cfg(not(target_arch = "wasm32"))]
pub use convention_endpoint::*;
#[cfg(not(target_arch = "wasm32"))]
pub use convention_file::ConventionFileGenerator;
pub use definition::*;
#[cfg(not(target_arch = "wasm32"))]
pub use form_state::FormStateExtractor;
pub use image::*;
#[cfg(not(target_arch = "wasm32"))]
pub use native_contract::*;
#[cfg(target_arch = "wasm32")]
pub use page_runtime::*;
pub use patch::*;
#[cfg(target_arch = "wasm32")]
pub use runtime_bridge::*;
pub use studio_contract::*;
pub use vm::*;

#[cfg(not(target_arch = "wasm32"))]
pub use patch_agent::ProgramPatchAgent;

rudi::enable! {}
