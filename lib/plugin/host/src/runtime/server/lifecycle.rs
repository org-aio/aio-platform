use super::{
    RuntimeState,
    repository::DiscoveredPlugin,
    store::{BoundRuntime, ProcessBinding},
    supervisor::ProcessInstance,
    wasm::WasmActivation,
};
use crate::runtime::PluginRuntime;

pub(super) async fn prepare_wasm(
    state: &RuntimeState,
    tenant_id: &str,
    plugin: &mut DiscoveredPlugin,
) -> anyhow::Result<WasmActivation> {
    let activation = prepare_wasm_revision(
        state,
        tenant_id,
        &plugin.source_id,
        &plugin.revision,
        &plugin.artifact,
    )
    .await?;
    plugin.pages.clone_from(&activation.pages);
    Ok(activation)
}

async fn prepare_wasm_revision(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    revision: &str,
    artifact: &str,
) -> anyhow::Result<WasmActivation> {
    let activation = state
        .activate_wasm(tenant_id, source_id, revision, artifact)
        .await?;
    if let Err(error) = state.repository.validate_pages(revision, &activation.pages) {
        if activation.created {
            let _ = state.wasm.deactivate(tenant_id, source_id, revision);
        }
        return Err(error);
    }
    Ok(activation)
}

pub(super) async fn prepare_bound_wasm(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    target: &BoundRuntime,
) -> anyhow::Result<WasmActivation> {
    let activation = prepare_wasm_revision(
        state,
        tenant_id,
        source_id,
        &target.revision,
        &target.artifact,
    )
    .await?;
    if let Err(error) = state
        .store
        .verify_revision_pages(&target.revision_id, &activation.pages)
        .await
    {
        cleanup_new_wasm(
            state,
            tenant_id,
            source_id,
            &target.revision,
            Some(&activation),
        )?;
        return Err(error);
    }
    Ok(activation)
}

pub(super) fn cleanup_new_wasm(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    revision: &str,
    activation: Option<&WasmActivation>,
) -> anyhow::Result<()> {
    if activation.is_some_and(|activation| activation.created) {
        state.wasm.deactivate(tenant_id, source_id, revision)?;
    }
    Ok(())
}

pub(super) fn deactivate_previous_wasm(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    previous_revision: Option<&str>,
    current_revision: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(previous_revision) = previous_revision
        && current_revision != Some(previous_revision)
    {
        state
            .wasm
            .deactivate(tenant_id, source_id, previous_revision)?;
    }
    Ok(())
}

pub(super) fn deactivate_bound_wasm(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    target: &BoundRuntime,
) -> anyhow::Result<()> {
    if target.runtime == PluginRuntime::WasmComponent {
        state
            .wasm
            .deactivate(tenant_id, source_id, &target.revision)?;
    }
    Ok(())
}

pub(super) async fn prepare_process(
    state: &RuntimeState,
    tenant_id: &str,
    plugin: &mut DiscoveredPlugin,
) -> anyhow::Result<ProcessInstance> {
    let (instance, pages) =
        prepare_process_revision(state, tenant_id, &plugin.source_id, &plugin.revision).await?;
    plugin.pages = pages;
    Ok(instance)
}

pub(super) async fn prepare_process_revision(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    revision: &str,
) -> anyhow::Result<(ProcessInstance, Vec<az_plugin_manifest::PageDefinition>)> {
    state.restore_package_cache(revision).await?;
    state.process.health().await?;
    let instance = state.process.start(tenant_id, source_id, revision).await?;
    let validation = async {
        let pages = state.process.load_pages(&instance.endpoint).await?;
        state.repository.validate_pages(revision, &pages)?;
        Ok::<_, anyhow::Error>(pages)
    }
    .await;
    match validation {
        Ok(pages) => Ok((instance, pages)),
        Err(error) => {
            if instance.created {
                let _ = state.process.stop(&instance.instance_id).await;
            }
            Err(error)
        }
    }
}

pub(super) async fn stop_previous_process(
    state: &RuntimeState,
    previous: Option<ProcessBinding>,
    current: Option<&ProcessInstance>,
) -> anyhow::Result<()> {
    if let Some(previous) = previous
        && current.is_none_or(|current| current.instance_id != previous.instance_id)
    {
        state.process.stop(&previous.instance_id).await?;
    }
    Ok(())
}

pub(super) async fn cleanup_new_process(
    state: &RuntimeState,
    instance: Option<&ProcessInstance>,
) -> anyhow::Result<()> {
    if let Some(instance) = instance
        && instance.created
    {
        state.process.stop(&instance.instance_id).await?;
    }
    Ok(())
}

pub(super) async fn restore_process_binding(
    state: &RuntimeState,
    tenant_id: &str,
    source_id: &str,
    target: Option<&BoundRuntime>,
) -> anyhow::Result<()> {
    let Some(target) = target else {
        return Ok(());
    };
    state.restore_package_cache(&target.revision).await?;
    if target.runtime != PluginRuntime::Process {
        return Ok(());
    }
    let instance = state
        .process
        .start(tenant_id, source_id, &target.revision)
        .await?;
    if let Err(error) = state
        .store
        .restore_process_instance(tenant_id, source_id, target, &instance)
        .await
    {
        if instance.created {
            let _ = state.process.stop(&instance.instance_id).await;
        }
        return Err(error);
    }
    Ok(())
}
