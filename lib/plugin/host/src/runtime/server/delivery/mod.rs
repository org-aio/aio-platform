pub use documents::validate_documentation;
mod contract;
pub use contract::{DeliveryArtifact, DeliveryService, PluginDocumentation};
pub(in crate::runtime::server) mod components;
mod controller;
mod discovery;
pub(crate) mod documents;
pub(crate) mod postgres;
pub(crate) mod postgres_documents;
mod rollout;
#[cfg(feature = "test-support")]
mod rollout_tests;
mod store;
#[cfg(test)]
mod tests;
#[cfg(feature = "test-support")]
pub(super) use rollout_tests::exercise_rollouts;

pub(super) use controller::router;
pub(super) use store::migrate;

pub(super) fn start(
    state: super::RuntimeState,
    tasks: std::sync::Arc<super::plugins::PluginTasks>,
) -> anyhow::Result<()> {
    if state.config.delivery.is_some() {
        tasks.spawn(discovery::run(state.clone(), tasks.cancellation()))?;
        tasks.spawn(rollout::run(state, tasks.cancellation()))?;
    }
    Ok(())
}

pub(super) use rollout::{ensure_current, exclude_revision, remember_installation};

pub(super) use components::finish_publication;
