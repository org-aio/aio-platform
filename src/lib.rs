#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "business/agent_config.rs"]
mod agent_config;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "business/application_compiler.rs"]
mod application_compiler;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "business/business_module.rs"]
mod business_module;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "domain/compiled_artifact.rs"]
mod compiled_artifact;
#[path = "compiler/compiler.rs"]
mod compiler;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "business/convention_endpoint.rs"]
mod convention_endpoint;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "business/convention_source.rs"]
mod convention_source;
#[path = "domain/definition.rs"]
mod definition;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "runtime/form_state.rs"]
mod form_state;
#[cfg(any(feature = "runtime-ui", test))]
#[path = "runtime/identifier_generation.rs"]
mod identifier_generation;
#[path = "domain/image.rs"]
mod image;
#[cfg(any(feature = "runtime-ui", test))]
#[path = "runtime/model_audit.rs"]
mod model_audit;
#[cfg(any(feature = "runtime-ui", test))]
#[path = "runtime/page_endpoint_draft.rs"]
mod page_endpoint_draft;
#[cfg(any(feature = "runtime-ui", test))]
#[path = "runtime/page_renderer_draft.rs"]
mod page_renderer_draft;
#[cfg(feature = "runtime-ui")]
#[path = "runtime/page_runtime.rs"]
mod page_runtime;
#[path = "patch/patch.rs"]
mod patch;
#[cfg(feature = "runtime-ui")]
#[path = "runtime/runtime_bridge.rs"]
mod runtime_bridge;
#[cfg(any(feature = "runtime-ui", test))]
#[path = "runtime/runtime_record_form.rs"]
mod runtime_record_form;
#[cfg(feature = "runtime-ui")]
#[path = "runtime/runtime_tree.rs"]
mod runtime_tree;
#[path = "domain/studio_contract.rs"]
mod studio_contract;
#[cfg(any(feature = "runtime-ui", test))]
#[path = "navigation/studio_navigation.rs"]
mod studio_navigation;
#[path = "execution/vm.rs"]
mod vm;

#[cfg(feature = "runtime-ui")]
#[path = "transport/browser_http.rs"]
pub mod browser_http;
#[cfg(feature = "runtime-ui")]
mod ui;
#[cfg(feature = "runtime-ui")]
pub use ui::StudioPage;

#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "domain/capability.rs"]
pub mod capability;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "patch/patch_agent.rs"]
mod patch_agent;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "execution/program_runtime.rs"]
pub mod program_runtime;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "storage/program_store.rs"]
pub mod program_store;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
#[path = "transport/studio_http.rs"]
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
