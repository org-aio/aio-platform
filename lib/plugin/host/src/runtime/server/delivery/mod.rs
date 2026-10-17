pub use documents::validate_documentation;
mod contract;
pub use contract::{DeliveryArtifact, DeliveryService, PluginDocumentation};
#[cfg(feature = "postgres")]
pub(in crate::runtime::server) mod components;
mod controller;
pub(crate) mod discovery;
mod sources;
pub use sources::{DeliverySource, DeliverySources, DeliveryTarget};
pub(crate) mod documents;
#[cfg(feature = "postgres")]
pub(crate) mod postgres;
#[cfg(feature = "postgres")]
pub(crate) mod postgres_documents;
#[cfg(feature = "postgres")]
pub(crate) mod postgres_sources;
#[cfg(feature = "postgres")]
mod publication;
#[cfg(feature = "postgres")]
mod rollout;
#[cfg(feature = "test-support")]
#[cfg(feature = "postgres")]
mod rollout_tests;
#[cfg(feature = "postgres")]
mod store;
#[cfg(all(test, feature = "postgres"))]
mod tests;
#[cfg(feature = "test-support")]
#[cfg(feature = "postgres")]
pub(super) use rollout_tests::exercise_rollouts;

pub(super) use controller::router;
#[cfg(feature = "postgres")]
pub(super) use store::migrate;

#[cfg(feature = "postgres")]
pub(super) fn start_rollout(
    state: super::RuntimeState,
    tasks: std::sync::Arc<super::plugins::PluginTasks>,
) -> anyhow::Result<()> {
    if state.config.delivery.is_some() {
        tasks.spawn(rollout::run(state, tasks.cancellation()))?;
    }
    Ok(())
}

#[cfg(feature = "postgres")]
pub(super) fn start_publication(
    state: super::RuntimeState,
    tasks: std::sync::Arc<super::plugins::PluginTasks>,
) -> anyhow::Result<()> {
    if state.config.delivery.is_some() {
        tasks.spawn(publication::run(state, tasks.cancellation()))?;
    }
    Ok(())
}

#[cfg(feature = "postgres")]
pub(super) use rollout::{ensure_current, exclude_revision, remember_installation};

#[cfg(feature = "postgres")]
pub(super) use components::finish_publication;
