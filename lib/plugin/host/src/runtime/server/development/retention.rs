use super::super::RuntimeState;
use anyhow::Result;

pub(super) async fn collect(state: &RuntimeState) -> Result<()> {
    let _mutation = state.development.mutation.lock().await;
    let mut protected = state.frontend.active_revisions()?;
    protected.extend(
        state
            .development
            .versions
            .read()
            .await
            .values()
            .map(|(_, revision)| revision.clone()),
    );
    let mut snapshots = Vec::new();
    for entry in std::fs::read_dir(&state.repository.cache_root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() == 64
            && name.bytes().all(|b| b.is_ascii_hexdigit())
            && entry.file_type()?.is_dir()
        {
            snapshots.push((entry.metadata()?.modified()?, name, entry.path()));
        }
    }
    snapshots.sort_by_key(|item| std::cmp::Reverse(item.0));
    // 保留最近十次构建，以及仍在运行或被浏览器引用的版本。
    for (_, revision, path) in snapshots.into_iter().skip(10) {
        if protected.contains(&revision) {
            continue;
        }
        std::fs::remove_dir_all(path)?;
        state.development.artifacts.write().await.remove(&revision);
        state.frontend.packages.lock().await.remove(&revision);
    }
    Ok(())
}
