mod deployment;
mod files;
mod package;
#[cfg(test)]
mod tests;

use super::{Store, storage::Installation};
use crate::InstallationPlan;
use anyhow::Result;

pub(super) fn install(
    store: &Store,
    plan: &InstallationPlan,
    record: &mut Installation,
) -> Result<()> {
    let files = package::sources(plan)?;
    deployment::apply(store, files, record)
}

pub(super) fn uninstall(store: &Store, record: &Installation) -> Result<()> {
    if record.skills.is_empty() {
        return Ok(());
    }
    files::remove(&store.skills_root, &record.skills)
}
