pub(in crate::runtime::server) mod components;
mod controller;
mod discovery;
mod documents;
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

pub(super) fn start(state: super::RuntimeState) {
    if state.config.delivery.is_some() {
        tokio::spawn(discovery::run(state.clone()));
        tokio::spawn(rollout::run(state));
    }
}

pub(super) use rollout::{ensure_current, exclude_revision, remember_installation};

pub(super) use components::finish_publication;
