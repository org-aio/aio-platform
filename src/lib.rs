#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod compiler;
#[cfg(not(target_arch = "wasm32"))]
mod convention_file;
mod definition;
#[cfg(not(target_arch = "wasm32"))]
mod form_state;
mod image;
#[cfg(target_arch = "wasm32")]
mod page_runtime;
mod patch;
mod studio_contract;
mod vm;

pub mod bootstrap;
pub use bootstrap::*;

#[cfg(target_arch = "wasm32")]
mod browser_bootstrap;
#[cfg(target_arch = "wasm32")]
mod browser_http;
#[cfg(target_arch = "wasm32")]
mod design_system;
#[cfg(target_arch = "wasm32")]
mod ui;
#[cfg(target_arch = "wasm32")]
mod workbench;

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

pub use compiler::{
    CompileFailure, CompilerStage, Diagnostic, DiagnosticSeverity, ProgramCompiler, compile_page,
    content_hash, convention_page_module_name, convention_page_path,
};
#[cfg(not(target_arch = "wasm32"))]
pub use convention_file::ConventionFileGenerator;
pub use definition::*;
#[cfg(not(target_arch = "wasm32"))]
pub use form_state::FormStateExtractor;
pub use image::*;
#[cfg(target_arch = "wasm32")]
pub use page_runtime::*;
pub use patch::*;
pub use studio_contract::*;
pub use vm::*;

#[cfg(target_arch = "wasm32")]
pub use workbench::App;

#[cfg(not(target_arch = "wasm32"))]
pub use patch_agent::ProgramPatchAgent;

rudi::enable! {}
