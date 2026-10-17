use super::*;
use az_plugin_host::runtime::{PluginRuntime, PublishState};
use uuid::Uuid;

#[tokio::test]
async fn file_catalog_recovers_pending_publications_and_enforces_credential_and_commit_boundaries()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("jobs.json");
    let catalog = FileCatalog::open(&path);
    let first = package("1.0.0")?;
    let second = package("2.0.0")?;
    let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, first.git.as_bytes()).to_string();
    let credential = catalog
        .create_publish_credential("tenant", &first.git)
        .await?;
    assert!(
        catalog
            .revoke_publish_credential("other", &credential.id)
            .await
            .is_err()
    );
    catalog.fail_commit.store(true, Ordering::SeqCst);
    assert!(catalog.save_package(&first).await.is_err());
    assert!(
        FileCatalog::open(&path)
            .package_archive(&first.rev)
            .await?
            .is_none()
    );
    for package in [&first, &second] {
        catalog.save_package(package).await?;
        assert!(catalog.downloadable_package(&package.rev).await?.is_none());
    }
    let first_job = catalog
        .queue_publish_job(
            "tenant",
            &source,
            &first.git,
            &first.rev,
            PluginRuntime::WasmComponent,
            1,
        )
        .await?;
    assert!(catalog.claim_publish_job(&first_job.id).await?);
    assert!(!catalog.claim_publish_job(&first_job.id).await?);
    let second_job = catalog
        .queue_publish_job(
            "tenant",
            &source,
            &second.git,
            &second.rev,
            PluginRuntime::WasmComponent,
            1,
        )
        .await?;
    drop(catalog);
    // 新目录对象、新宿主和空产物缓存，只从持久任务与包恢复。
    let catalog = FileCatalog::open(&path);
    let mut host =
        Host::start_selected(root.path(), "invalid database", selected(catalog.clone())).await?;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let first = catalog
                .publish_job("tenant", &first_job.id)
                .await?
                .context("旧任务丢失")?;
            let second = catalog
                .publish_job("tenant", &second_job.id)
                .await?
                .context("新任务丢失")?;
            if first.state == PublishState::Failed && second.state == PublishState::Active {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    assert!(
        catalog
            .publish_job("other", &second_job.id)
            .await?
            .is_none()
    );
    assert_eq!(
        get(&host, &format!("/api/runtime/services/{source}/echo")).await?["revision"],
        second.rev
    );
    assert!(catalog.downloadable_package(&first.rev).await?.is_none());
    let token_client = reqwest::Client::builder().no_proxy().build()?;
    let url = format!("{}/api/runtime/publish-jobs/{}", host.base, second_job.id);
    assert!(
        token_client
            .get(&url)
            .bearer_auth(&credential.token)
            .send()
            .await?
            .status()
            .is_success()
    );
    let foreign = catalog
        .create_publish_credential("tenant", "https://github.com/example/foreign.git")
        .await?;
    assert_eq!(
        token_client
            .get(&url)
            .bearer_auth(&foreign.token)
            .send()
            .await?
            .status(),
        403
    );
    let revoke = host
        .client
        .delete(format!(
            "{}/api/runtime/publish-credentials/{}",
            host.base, credential.id
        ))
        .send()
        .await?;
    assert_eq!(revoke.status(), 204);
    assert_eq!(
        token_client
            .get(&url)
            .bearer_auth(&credential.token)
            .send()
            .await?
            .status(),
        401
    );

    // 在线替换目录实例必须重建管理插件；随后仍能读取相同文件与安装版本。
    let replacement = FileCatalog::open(&path);
    host.reconfigure(selected(replacement.clone())).await?;
    assert!(Arc::ptr_eq(
        &replacement,
        &host.state.service::<FileCatalog>()?
    ));
    assert_eq!(
        get(&host, &format!("/api/runtime/services/{source}/echo")).await?["revision"],
        second.rev
    );
    host.stop().await?;
    Ok(())
}
