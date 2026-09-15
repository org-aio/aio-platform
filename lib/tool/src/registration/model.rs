use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub command: String,
    #[serde(default)]
    pub metadata: Metadata,
    pub platforms: Vec<String>,
    #[serde(default)]
    pub uninstall: String,
    #[serde(default)]
    pub detect: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    #[serde(default)]
    pub git: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Documentation {
    pub metadata: Metadata,
    pub readme: String,
    pub link_base: String,
    pub image_base: String,
    pub error: Option<String>,
}
