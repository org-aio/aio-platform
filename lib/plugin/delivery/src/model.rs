use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryManifest {
    pub version: u32,
    pub build: BuildRecipe,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRecipe {
    pub environment: BuildEnvironment,
    pub command: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildEnvironment {
    Rust,
    Kotlin,
    #[serde(rename = "typescript")]
    TypeScript,
    Fullstack,
}

impl BuildEnvironment {
    pub fn image_variable(self) -> &'static str {
        match self {
            Self::Rust => "AIO_BUILD_IMAGE_RUST",
            Self::Kotlin => "AIO_BUILD_IMAGE_KOTLIN",
            Self::TypeScript => "AIO_BUILD_IMAGE_TYPESCRIPT",
            Self::Fullstack => "AIO_BUILD_IMAGE_FULLSTACK",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildJob {
    pub id: i64,
    pub lease: String,
    pub git: String,
    pub source_revision: String,
    pub version: String,
    pub recipe: BuildRecipe,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Documentation {
    pub readme: String,
    pub images: BTreeMap<String, DocumentImage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentImage {
    pub content_base64: String,
    pub content_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BuildReport {
    pub lease: String,
    pub error: Option<String>,
    pub documentation: Documentation,
}
