use anyhow::{Context as _, Result};

use super::{
    RuntimeState,
    lifecycle::{
        cleanup_new_process, cleanup_new_wasm, deactivate_previous_wasm, prepare_process,
        prepare_wasm, restore_process_binding, stop_previous_process,
    },
    repository::{DiscoveredPlugin, RepositoryInstaller},
};
use crate::runtime::{InstallPluginRequest, MarketplaceEntry, PluginRuntime};

pub(super) struct ActivatedPlugin {
    pub source_id: String,
    pub revision: String,
    pub page_count: usize,
}

pub(super) async fn install(
    state: &RuntimeState,
    tenant_id: &str,
    request: &InstallPluginRequest,
) -> Result<ActivatedPlugin> {
    if let Some(components) = &state.components
        && let Some((source, bundle)) = components
            .published(&request.git, request.rev.as_deref())
            .await?
    {
        components
            .install(tenant_id, &request.git, request.rev.as_deref())
            .await?;
        return Ok(ActivatedPlugin {
            source_id: source.to_string(),
            revision: bundle.digest,
            page_count: components.page_count(tenant_id, source).await?,
        });
    }
    let publication = state
        .store
        .published_marketplace_entry(&request.git, request.rev.as_deref())
        .await
        .context("查找数据库已发布插件失败")?;
    if let Some(publication) = &publication {
        state.restore_package_cache(&publication.rev).await?;
    }
    let (discovered, validation_detail) =
        resolve_install_candidate(&state.repository, request, publication.as_ref()).await?;
    activate(state, tenant_id, discovered, validation_detail, None).await
}

async fn resolve_install_candidate(
    repository: &RepositoryInstaller,
    request: &InstallPluginRequest,
    publication: Option<&MarketplaceEntry>,
) -> Result<(DiscoveredPlugin, &'static str)> {
    if let Some(publication) = publication {
        let discovered = repository
            .validate_published(&publication.git, &publication.rev)
            .await
            .context("校验数据库已发布插件的本地 artifact 失败")?;
        return Ok((
            discovered,
            "已校验数据库已发布 artifact 和市场元数据（未访问 Git）",
        ));
    }
    let discovered = repository
        .discover(&request.git, request.rev.as_deref())
        .await
        .context("从受限 Git 来源发现插件失败")?;
    Ok((discovered, "已校验 Git 完整提交、清单和预构建 artifact"))
}

pub(super) async fn activate(
    state: &RuntimeState,
    tenant_id: &str,
    discovered: DiscoveredPlugin,
    validation_detail: &str,
    publication: Option<&MarketplaceEntry>,
) -> Result<ActivatedPlugin> {
    activate_inner(
        state,
        tenant_id,
        discovered,
        validation_detail,
        publication,
        false,
    )
    .await
}

pub(super) async fn install_followed(
    state: &RuntimeState,
    tenant_id: &str,
    request: &InstallPluginRequest,
) -> Result<ActivatedPlugin> {
    let publication = state
        .store
        .published_marketplace_entry(&request.git, request.rev.as_deref())
        .await?
        .context("自动更新版本尚未发布")?;
    state.restore_package_cache(&publication.rev).await?;
    let (discovered, detail) =
        resolve_install_candidate(&state.repository, request, Some(&publication)).await?;
    activate_inner(state, tenant_id, discovered, detail, None, true).await
}

async fn activate_inner(
    state: &RuntimeState,
    tenant_id: &str,
    mut discovered: DiscoveredPlugin,
    validation_detail: &str,
    publication: Option<&MarketplaceEntry>,
    following: bool,
) -> Result<ActivatedPlugin> {
    if let Some(source_id) = state.store.source_id(&discovered.git).await? {
        discovered.source_id = source_id;
    }
    let activation_lock = state.activation_lock(tenant_id, &discovered.source_id)?;
    let _activation_guard = activation_lock.lock().await;
    if following {
        let eligible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenant_plugin_bindings b JOIN plugin_sources s ON s.id=b.source_id JOIN marketplace_entries m ON m.git=s.git AND m.source='aio://published' LEFT JOIN delivery_installations i ON i.tenant_id=b.tenant_id AND i.source_id=b.source_id WHERE b.tenant_id=$1 AND b.source_id=$2 AND b.enabled AND m.rev=$3 AND i.excluded_revision IS DISTINCT FROM $3)")
            .bind(tenant_id).bind(&discovered.source_id).bind(&discovered.revision).fetch_one(&state.store.pool).await?;
        anyhow::ensure!(eligible, "安装状态或目标版本已改变，取消自动升级");
    }
    let publish_only = if publication.is_some() {
        sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM delivery_jobs WHERE package_revision=$3) AND EXISTS(SELECT 1 FROM delivery_installations WHERE tenant_id=$1 AND source_id=$2) AND NOT EXISTS(SELECT 1 FROM tenant_plugin_bindings WHERE tenant_id=$1 AND source_id=$2 AND enabled)")
            .bind(tenant_id).bind(&discovered.source_id).bind(&discovered.revision).fetch_one(&state.store.pool).await?
    } else {
        false
    };
    let current_revision = discovered.revision.clone();
    state
        .store
        .record_lifecycle_event(
            tenant_id,
            &discovered.source_id,
            None,
            "validate",
            validation_detail,
        )
        .await?;
    let previous = state
        .store
        .active_process(tenant_id, &discovered.source_id)
        .await?;
    let previous_target = if previous.is_some() {
        Some(
            state
                .store
                .bound_runtime(tenant_id, &discovered.source_id)
                .await?,
        )
    } else {
        None
    };
    let previous_wasm = state
        .store
        .active_wasm_revision(tenant_id, &discovered.source_id)
        .await?;
    let instance = if discovered.runtime == PluginRuntime::Process {
        Some(prepare_process(state, tenant_id, &mut discovered).await?)
    } else {
        None
    };
    let wasm = if discovered.runtime == PluginRuntime::WasmComponent {
        Some(prepare_wasm(state, tenant_id, &mut discovered).await?)
    } else {
        None
    };
    if (instance.is_some() || wasm.is_some())
        && let Err(error) = state
            .store
            .record_lifecycle_event(
                tenant_id,
                &discovered.source_id,
                None,
                "health-check",
                "运行时健康检查通过",
            )
            .await
    {
        let _ = cleanup_new_process(state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            state,
            tenant_id,
            &discovered.source_id,
            &current_revision,
            wasm.as_ref(),
        );
        return Err(error);
    }
    let activated = ActivatedPlugin {
        source_id: discovered.source_id.clone(),
        revision: discovered.revision.clone(),
        page_count: discovered.pages.len(),
    };
    if publish_only {
        let result = state
            .store
            .publish_only(tenant_id, discovered, publication.expect("发布元数据"))
            .await;
        let _ = cleanup_new_process(state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            state,
            tenant_id,
            &activated.source_id,
            &current_revision,
            wasm.as_ref(),
        );
        result?;
        return Ok(activated);
    }
    if let Err(error) = state
        .store
        .activate(tenant_id, discovered, instance.as_ref(), publication)
        .await
    {
        let _ = cleanup_new_process(state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            state,
            tenant_id,
            &activated.source_id,
            &current_revision,
            wasm.as_ref(),
        );
        if let Err(recovery) = restore_process_binding(
            state,
            tenant_id,
            &activated.source_id,
            previous_target.as_ref(),
        )
        .await
        {
            return Err(anyhow::anyhow!(
                "激活插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            ));
        }
        return Err(error);
    }
    if let Err(error) = stop_previous_process(state, previous, instance.as_ref()).await {
        eprintln!("新版本已切换，旧进程清理待重试: {error:#}");
    }
    deactivate_previous_wasm(
        state,
        tenant_id,
        &activated.source_id,
        previous_wasm.as_deref(),
        wasm.as_ref().map(|_| current_revision.as_str()),
    )?;
    Ok(activated)
}

#[cfg(test)]
#[path = "installation_tests.rs"]
mod tests;
