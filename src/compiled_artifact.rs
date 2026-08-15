use std::{fs, path::PathBuf, process};

use anyhow::{Context, Result};
use serde_json::json;

use crate::ProgramImage;

#[derive(Clone, Debug)]
pub struct CompiledArtifactWriter {
    root_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledArtifactResult {
    pub program_image: PathBuf,
    pub endpoints: PathBuf,
}

impl CompiledArtifactWriter {
    #[must_use]
    pub fn workspace_target() -> Self {
        let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        Self {
            root_dir: workspace_dir.join("target/aio/studio"),
        }
    }

    #[must_use]
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    pub fn write(&self, image: &ProgramImage) -> Result<CompiledArtifactResult> {
        let artifact_dir = self
            .root_dir
            .join(artifact_segment(&image.name))
            .join(artifact_segment(&image.content_hash));
        fs::create_dir_all(&artifact_dir)
            .with_context(|| format!("创建 Studio 编译产物目录失败: {}", artifact_dir.display()))?;
        let program_image = artifact_dir.join("program-image.json");
        let endpoints = artifact_dir.join("endpoints.json");
        let image_bytes = serde_json::to_vec_pretty(image).context("序列化 ProgramImage 失败")?;
        write_atomic(&program_image, &image_bytes)?;

        let endpoint_values = image
            .pages
            .values()
            .flat_map(|page| {
                page.endpoints.iter().map(|endpoint| {
                    json!({
                        "page_id": page.id,
                        "page_name": page.name,
                        "page_title": page.title,
                        "endpoint": endpoint,
                    })
                })
            })
            .collect::<Vec<_>>();
        let endpoint_bytes =
            serde_json::to_vec_pretty(&endpoint_values).context("序列化编译接口产物失败")?;
        write_atomic(&endpoints, &endpoint_bytes)?;

        Ok(CompiledArtifactResult {
            program_image,
            endpoints,
        })
    }
}

fn artifact_segment(value: &str) -> String {
    let segment = value
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '-' | '_') {
                value
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.is_empty() {
        "program".to_owned()
    } else {
        segment
    }
}

fn write_atomic(path: &PathBuf, contents: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", process::id()));
    fs::write(&temporary, contents)
        .with_context(|| format!("写入 Studio 临时编译产物失败: {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("提交 Studio 编译产物失败: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        CompiledPage, CompiledPageEndpoint, CompiledPageRenderer, DefinitionState, ImageTarget,
        MenuDefinition, PROGRAM_SCHEMA_VERSION, PageEndpointSource, RestMethod, SymbolId,
    };

    use super::*;

    fn image() -> ProgramImage {
        let page_id = SymbolId::new();
        let endpoint = CompiledPageEndpoint {
            id: "builtin-list".to_owned(),
            title: "查询".to_owned(),
            description: "模型默认查询".to_owned(),
            method: RestMethod::Get,
            path: "/api/runtime/models/assets/records".to_owned(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            source: PageEndpointSource::BuiltIn,
        };
        ProgramImage {
            schema_version: PROGRAM_SCHEMA_VERSION,
            compiler_version: "test".to_owned(),
            content_hash: "content-hash".to_owned(),
            program_id: SymbolId::new(),
            name: "aio/test".to_owned(),
            title: "AIO".to_owned(),
            revision_id: "revision".to_owned(),
            target: ImageTarget::Universal,
            menus: vec![MenuDefinition {
                id: SymbolId::new(),
                name: "root".to_owned(),
                title: "根".to_owned(),
                state: DefinitionState::Known,
                icon: None,
                page_id: None,
                enabled: true,
                children: Vec::new(),
                required_permissions: Vec::new(),
                row_actions: crate::MenuRowActions::default(),
            }],
            permissions: Vec::new(),
            pages: BTreeMap::from([(
                page_id,
                CompiledPage {
                    id: page_id,
                    name: "assets".to_owned(),
                    title: "资产".to_owned(),
                    renderer: CompiledPageRenderer::ConventionFile {
                        module_name: "assets".to_owned(),
                        expected_path: "src/pages/assets.rs".to_owned(),
                    },
                    endpoints: vec![endpoint],
                },
            )]),
            client_functions: BTreeMap::new(),
            server_functions: BTreeMap::new(),
            models: BTreeMap::new(),
            routes: Vec::new(),
            dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn writes_program_image_and_builtin_endpoints_only_under_target_root() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "aio-compiled-artifact-{}-{}",
            process::id(),
            az_plugin_core::timestamp_ms()
        ));
        let writer = CompiledArtifactWriter::new(root.clone());

        let result = writer.write(&image())?;

        assert!(result.program_image.starts_with(&root));
        assert!(result.endpoints.starts_with(&root));
        let endpoints: serde_json::Value = serde_json::from_slice(&fs::read(&result.endpoints)?)?;
        assert_eq!(endpoints[0]["endpoint"]["source"], "built_in");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
