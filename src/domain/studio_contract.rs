use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ApplicationTarget, CapabilityCatalog, ProgramDefinition};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplicationSourceFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApplicationBundle {
    pub application_id: String,
    pub title: String,
    pub revision_id: String,
    pub content_hash: String,
    pub targets: Vec<ApplicationTarget>,
    pub files: Vec<ApplicationSourceFile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplicationGenerationResult {
    pub application_id: String,
    pub path: String,
    pub revision_id: String,
    pub content_hash: String,
    pub targets: Vec<ApplicationTarget>,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StudioCatalog {
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
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub struct StudioPage<T> {
    pub d: Vec<T>,
    pub t: u64,
    pub p: StudioPageParams,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftSnapshot {
    pub program_id: String,
    pub version: i64,
    pub definition: ProgramDefinition,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevisionSnapshot {
    pub id: String,
    pub program_id: String,
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
    pub program_id: String,
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
    pub program_id: String,
    pub base_version: i64,
    pub status: String,
    pub final_revision_id: Option<String>,
    pub diagnostics: Value,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRecordInput {
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormStateExtractionRequest {
    pub prompt: String,
    pub current_form_state: Value,
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormStateExtractionResponse {
    pub form_state: Value,
    pub model: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRecordFilterOperator {
    Equals,
    Contains,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRecordFilter {
    pub field: String,
    pub operator: RuntimeRecordFilterOperator,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRecordSortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRecordSort {
    pub field: String,
    pub direction: RuntimeRecordSortDirection,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRecordCriteria {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all: Vec<RuntimeRecordFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any: Vec<RuntimeRecordFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<RuntimeRecordSort>,
}

impl RuntimeRecordCriteria {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.all.is_empty() && self.any.is_empty() && self.sort.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRecordView {
    pub id: String,
    pub payload: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRecordPage {
    pub d: Vec<RuntimeRecordView>,
    pub t: u64,
    pub p: StudioPageParams,
}
