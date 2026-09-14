use super::super::{
    RuntimeState, frontend_package::FrontendPackage,
    publication_validation::ensure_publish_capabilities, repository::DiscoveredPlugin,
    supervisor::ProcessInstance,
};
use crate::runtime::PluginRuntime;
use anyhow::{Context, Result, ensure};
use az_plugin_development::DevArtifact;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

pub(super) async fn activate(state: &RuntimeState, artifact: DevArtifact) -> Result<String> {
    let session = state
        .config
        .development
        .as_ref()
        .context("仅开发宿主允许本地激活")?;
    let workspace = artifact.workspace.canonicalize()?;
    ensure!(
        session
            .workspaces
            .iter()
            .any(|entry| entry.path == workspace),
        "工作区不在本次运行集合内"
    );
    ensure!(
        artifact.source == az_plugin_development::source_identity(&workspace)?,
        "开发来源不匹配"
    );
    for digest in [&artifact.content_digest, &artifact.backend_digest] {
        ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "开发内容摘要无效"
        );
    }
    for path in [&artifact.frontend, &artifact.backend] {
        ensure!(
            path.canonicalize()?.starts_with(&workspace),
            "开发产物越出工作区"
        );
    }
    let _mutation = state.development.mutation.lock().await;
    let previous = state
        .development
        .versions
        .read()
        .await
        .get(&artifact.source)
        .cloned();
    ensure!(
        previous
            .as_ref()
            .is_none_or(|(generation, _)| *generation <= artifact.generation),
        "构建已过期"
    );
    let root = super::snapshot::prepare(state, &artifact)?;
    let revision = artifact.content_digest.clone();
    let text = std::fs::read_to_string(root.join("aio-plugin.toml"))?;
    let schema: toml::Value = toml::from_str(&text)?;
    if schema
        .get("schema_version")
        .and_then(|value| value.as_integer())
        == Some(2)
    {
        let bundle =
            az_plugin_bundle::VerifiedBundle::from_development_directory(&root, revision.clone())?;
        super::snapshot::dependencies(
            session,
            &bundle.manifest().plugin.dependencies,
            bundle
                .manifest()
                .plugin
                .marketplace
                .as_ref()
                .and_then(|m| m.parent.as_deref()),
        )?;
        let repository = &session
            .workspaces
            .iter()
            .find(|entry| entry.path == workspace)
            .context("工作区不存在")?
            .source;
        state
            .components()?
            .activate_development(&artifact, Arc::new(bundle), repository)
            .await?;
        state
            .development
            .versions
            .write()
            .await
            .insert(artifact.source, (artifact.generation, revision.clone()));
        return Ok(revision);
    }
    let manifest = az_plugin_manifest::read_manifest(&root)?;
    az_plugin_manifest::validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
    ensure_publish_capabilities(&manifest)?;
    super::snapshot::dependencies(session, &manifest.plugin.dependencies, None)?;
    let runtime = manifest
        .plugin
        .runtime
        .as_ref()
        .context("插件缺少运行时声明")?;
    let frontend = manifest
        .plugin
        .frontend
        .as_ref()
        .context("插件缺少前端声明")?;
    let source_id =
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, artifact.source.as_bytes()).to_string();
    let (pages, process) = match runtime.kind {
        PluginRuntime::Process => {
            let endpoint = artifact.endpoint.as_deref().context("开发进程尚未启动")?;
            let pages = state.process.load_pages(endpoint).await?;
            (
                pages,
                Some(ProcessInstance {
                    instance_id: artifact.backend_digest.clone(),
                    endpoint: endpoint.into(),
                    created: false,
                }),
            )
        }
        PluginRuntime::WasmComponent => {
            // 只有前端变化时复用同一后端 Store，保留内存状态。
            let pages = state
                .wasm
                .activate(
                    "development",
                    &source_id,
                    &artifact.backend_digest,
                    &root.join(&runtime.artifact),
                )?
                .pages;
            state.wasm.bind_revision(
                "development",
                &source_id,
                &artifact.backend_digest,
                &revision,
            )?;
            (pages, None)
        }
        PluginRuntime::PageDefinition => (
            az_plugin_manifest::parse_page_definitions(&std::fs::read(
                root.join(&runtime.artifact),
            )?)?,
            None,
        ),
        PluginRuntime::RustSource => anyhow::bail!("系统源码插件不属于运行时开发沙箱"),
    };
    state.repository.validate_pages(&revision, &pages)?;
    let files = az_plugin_manifest::frontend_files(&root, &manifest)?;
    let mut assets = BTreeMap::new();
    let mut asset_sizes = BTreeMap::new();
    for (name, path) in files {
        let bytes = std::fs::read(path)?;
        assets.insert(name.clone(), format!("{:x}", Sha256::digest(&bytes)));
        asset_sizes.insert(name, bytes.len());
    }
    state.development.artifacts.write().await.insert(
        revision.clone(),
        Arc::new(FrontendPackage {
            path: frontend.path.clone(),
            assets,
            asset_sizes,
        }),
    );
    state
        .store
        .activate(
            "development",
            DiscoveredPlugin {
                source_id,
                git: artifact.source.clone(),
                revision: revision.clone(),
                runtime: runtime.kind,
                manifest: serde_json::to_value(&manifest.plugin)?,
                pages,
                artifact: runtime.artifact.clone(),
            },
            process.as_ref(),
            None,
        )
        .await?;
    if runtime.kind == PluginRuntime::WasmComponent {
        state.wasm.retain_development_revisions(
            "development",
            &uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, artifact.source.as_bytes()).to_string(),
            &artifact.backend_digest,
            &revision,
        )?;
    }
    state
        .development
        .versions
        .write()
        .await
        .insert(artifact.source, (artifact.generation, revision.clone()));
    Ok(revision)
}

pub(super) async fn prepare(
    state: &RuntimeState,
    artifact: DevArtifact,
) -> Result<az_plugin_development::DevLaunch> {
    let session = state
        .config
        .development
        .as_ref()
        .context("开发会话不存在")?;
    let workspace = artifact.workspace.canonicalize()?;
    ensure!(
        session
            .workspaces
            .iter()
            .any(|entry| entry.path == workspace),
        "工作区不在本次运行集合内"
    );
    ensure!(
        artifact.source == az_plugin_development::source_identity(&workspace)?,
        "开发来源不匹配"
    );
    for path in [&artifact.frontend, &artifact.backend] {
        ensure!(
            path.canonicalize()?.starts_with(&workspace),
            "开发产物越出工作区"
        );
    }
    let _mutation = state.development.mutation.lock().await;
    let text = std::fs::read_to_string(workspace.join("aio-plugin.toml"))?;
    let manifest: toml::Value = toml::from_str(&text)?;
    if manifest.get("schema_version").and_then(|v| v.as_integer()) == Some(2) {
        let root = super::snapshot::prepare(&state, &artifact)?;
        let bundle = az_plugin_bundle::VerifiedBundle::from_development_directory(
            &root,
            artifact.content_digest.clone(),
        )?;
        super::snapshot::dependencies(
            session,
            &bundle.manifest().plugin.dependencies,
            bundle
                .manifest()
                .plugin
                .marketplace
                .as_ref()
                .and_then(|m| m.parent.as_deref()),
        )?;
        if bundle.manifest().plugin.runtime.process.is_some() {
            return state
                .components()?
                .prepare_development(&artifact, std::sync::Arc::new(bundle))
                .await;
        }
    }
    Ok(Default::default())
}
