use az_tool::publication::ReleaseMetadata;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct NpmPackage {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub engines: std::collections::BTreeMap<String, String>,
    pub bin: serde_json::Value,
    pub repository: serde_json::Value,
    pub aio: ReleaseMetadata,
    pub dist: Distribution,
}

#[derive(Deserialize)]
pub(super) struct Distribution {
    pub integrity: String,
}

#[derive(Deserialize)]
pub(super) struct Repository {
    pub default_branch: String,
}
