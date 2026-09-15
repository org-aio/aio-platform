use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolManifest {
    pub id: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub homepage: String,
    pub license: String,
    pub tags: Vec<String>,
    pub platforms: BTreeMap<String, InstallationPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationPlan {
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    pub install: Vec<CommandSpec>,
    #[serde(default)]
    pub uninstall: Vec<CommandSpec>,
    pub detect: Option<CommandSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub label: String,
    pub check: CommandSpec,
    pub version: Option<String>,
    pub help: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}
