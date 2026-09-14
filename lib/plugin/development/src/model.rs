use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevConfiguration {
    pub version: u32,
    pub plugin: Option<DevPlugin>,
    pub prepare: Option<BuildTask>,
    pub frontend: BuildTask,
    pub backend: BuildTask,
    pub run: Option<RunTask>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DevPlugin {
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildTask {
    pub inputs: Vec<String>,
    pub command: Vec<String>,
    pub output: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunTask {
    pub command: Vec<String>,
    #[serde(default)]
    pub debug_arguments: Vec<String>,
    #[serde(default = "health")]
    pub health: String,
}

fn health() -> String {
    "/health".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevArtifact {
    pub workspace: PathBuf,
    pub source: String,
    pub content_digest: String,
    pub frontend: PathBuf,
    pub backend: PathBuf,
    pub backend_digest: String,
    pub endpoint: Option<String>,
    pub generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevHostSession {
    pub database_url: String,
    pub root: PathBuf,
    pub port: u16,
    pub token: String,
    pub workspaces: Vec<DevWorkspace>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevWorkspace {
    pub path: PathBuf,
    pub source: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevStatus {
    pub generation: u64,
    pub phase: String,
    pub message: String,
    pub revisions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevelopmentLock {
    pub version: u32,
    pub host_version: String,
    pub plugins: Vec<LockedPlugin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedRelease {
    pub git: String,
    pub version: String,
    pub source_sha: String,
    pub digest: String,
    pub manifest: String,
    pub abi: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockedPlugin {
    pub source: String,
    pub workspace: Option<PathBuf>,
    pub source_sha: Option<String>,
    pub content_digest: String,
    pub package_digest: Option<String>,
    pub version: Option<String>,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DevLaunch {
    pub environment: BTreeMap<String, String>,
    pub socket: Option<PathBuf>,
}
