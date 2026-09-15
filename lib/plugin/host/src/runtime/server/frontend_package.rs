use super::RuntimeState;
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, sync::Arc};

pub(super) struct FrontendPackage {
    pub path: String,
    pub assets: BTreeMap<String, String>,
    pub asset_sizes: BTreeMap<String, usize>,
}

pub(super) async fn prepare(
    state: &RuntimeState,
    revision: &str,
    entry: &str,
) -> Result<Arc<FrontendPackage>> {
    // 缓存只保留校验后的不可变元数据；并发首次挂载共享一次整包准备。
    let mut cache = state.frontend.packages.lock().await;
    if let Some(package) = cache.get(revision)
        && state
            .repository
            .artifact(revision, &format!("{}/{entry}", package.path))
            .is_ok()
    {
        ensure!(
            package.assets.contains_key(entry),
            "前端入口不属于已发布版本"
        );
        return Ok(package.clone());
    }
    if let Some(package) = state.development.frontend(revision).await {
        ensure!(package.assets.contains_key(entry), "开发前端未声明入口");
        return Ok(package);
    }
    let archive = state
        .store
        .package_archive(revision)
        .await?
        .context("前端挂载要求完整二进制包")?;
    let package =
        tokio::task::spawn_blocking(move || az_plugin_package::PluginPackage::decode(&archive))
            .await
            .context("等待前端包校验失败")??;
    ensure!(
        package.rev == revision && package.frontend.contains_key(entry),
        "前端入口与活动版本不一致"
    );
    let verified = package.verify()?;
    let path = verified
        .manifest
        .plugin
        .frontend
        .context("插件未声明前端产物")?
        .path;
    state.repository.stage_publish(&package).await?;
    let metadata = Arc::new(FrontendPackage {
        path,
        asset_sizes: verified
            .frontend
            .into_iter()
            .map(|(path, bytes)| (path, bytes.len()))
            .collect(),
        assets: package
            .frontend
            .into_iter()
            .map(|(path, asset)| (path, asset.sha256))
            .collect(),
    });
    if cache.len() >= 64
        && let Some(oldest) = cache.keys().next().cloned()
    {
        cache.remove(&oldest);
    }
    cache.insert(revision.to_owned(), metadata.clone());
    Ok(metadata)
}
