#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod compiler;
mod component;
mod component_index;
mod component_kind;
mod components;
mod definition;
mod image;
mod patch;
mod renderer;
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
#[cfg(target_arch = "wasm32")]
mod workflow;

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
    CompileFailure, CompilerStage, Diagnostic, DiagnosticSeverity, ProgramCompiler, content_hash,
    preview_render_plan,
};
pub use component::{
    ComponentCatalogEntry, ComponentDefinition, ComponentEventSpec, ComponentPropertySpec,
    ComponentRenderContext, ComponentSpec, DynDynamicComponentProvider, DynamicComponentEvent,
    DynamicComponentProvider,
};
pub use component_index::{ComponentIndex, IndexedComponent};
pub use component_kind::{ComponentBehavior, ComponentPropertyKind, ComponentShape};
pub use definition::*;
pub use image::*;
pub use patch::*;
pub use renderer::{DynamicRenderData, DynamicRenderer};
pub use studio_contract::*;
pub use vm::*;

#[cfg(target_arch = "wasm32")]
pub use workbench::App;

#[cfg(not(target_arch = "wasm32"))]
pub use patch_agent::ProgramPatchAgent;

rudi::enable! {}
