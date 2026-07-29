use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CapabilityCatalog, ComponentCatalog, ProgramDefinition};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StudioCatalog {
    pub components: ComponentCatalog,
    pub capabilities: CapabilityCatalog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StudioPageParams {
    pub o: usize,
    pub s: usize,
}

impl Default for StudioPageParams {
    fn default() -> Self {
        Self { o: 0, s: 50 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudioPage<T> {
    pub d: Vec<T>,
    pub t: u64,
    pub p: StudioPageParams,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplicationSummary {
    pub id: String,
    pub name: String,
    pub title: String,
    pub active_revision_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateApplicationInput {
    pub name: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftSnapshot {
    pub application_id: String,
    pub version: i64,
    pub definition: ProgramDefinition,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevisionSnapshot {
    pub id: String,
    pub application_id: String,
    pub revision: i64,
    pub definition: ProgramDefinition,
    pub content_hash: String,
    pub origin: String,
    pub diagnostics: Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevisionRunSnapshot {
    pub id: String,
    pub application_id: String,
    pub revision_id: Option<String>,
    pub status: String,
    pub stage: String,
    pub diagnostics: Value,
    pub tests: Value,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VibeSessionSnapshot {
    pub id: String,
    pub application_id: String,
    pub base_version: i64,
    pub status: String,
    pub final_revision_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VibeMessageInput {
    pub role: String,
    pub prompt: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub patch: Option<Value>,
    pub diagnostics: Value,
    pub tests: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VibeRunRequest {
    pub prompt: String,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VibeRunAccepted {
    pub session_id: String,
    pub status: String,
}
