//! 下载使用真实整包和 HTTP 响应，操作继续核验活动挂载、信任和权限。

use super::*;

async fn action(host: &Host, token: &str, value: Value) -> Result<reqwest::Response> {
    host.post(&format!("/api/runtime/components/{token}/action"), value)
        .await
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn management_actions_and_large_downloads_keep_live_authority() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/manager-actions-{tenant}.git");
    let identity = Arc::new(Identity {
        tenant: tenant.clone(),
        revoked: false.into(),
        expired: false.into(),
        publisher: true.into(),
    });
    let probes: [Arc<Probe>; 2] = Default::default();
    let seed = manager(root.path(), &git, "1.0.0")?;
    let package_root = root.path().join("1.0.0");
    std::fs::write(
        package_root.join("aio-plugin.toml"),
        seed.manifest
            .replace("[plugin]\n", "[plugin]\nsettings_page='home'\n"),
    )?;
    // 不可压缩的负载确保下载确实越过 JSON 管理通道的 32 MiB 上限。
    let mut bytes = vec![0_u8; 34 * 1024 * 1024];
    let mut random = 0x12345678abcdef01_u64;
    for chunk in bytes.chunks_exact_mut(8) {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        chunk.copy_from_slice(&random.to_le_bytes());
    }
    std::fs::write(package_root.join("web/large.bin"), bytes)?;
    let first = Bundle::from_directory(
        &package_root,
        "aio-plugin.toml",
        git.clone(),
        seed.commit,
        seed.version,
    )?;
    let archive = first.encode()?;
    assert!(archive.len() > 32 * 1024 * 1024);
    let mut selected = selection(&identity, &probes, &[&first]);
    selected.extend([
        Arc::new(ToolsPlugin) as DynPlugin<RuntimeAssembly>,
        Arc::new(WorkerServicePlugin),
    ]);
    let mut host = Host::start_selected(root.path(), &database, selected).await?;
    let published = success(host.publish(&first).await?).await?;
    let source: Uuid = published["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .parse()?;
    success(host.install(&first).await?).await?;
    let token = host.mount(source).await?;
    let download = format!(
        "{}/api/runtime/components/{token}/packages/{}",
        host.base, first.digest
    );
    let settings = json!({"kind":"open-settings","page_id":format!("component:{source}:home")});
    let approved = success(action(&host, &token, settings.clone()).await?).await?;
    assert_eq!(approved["data"], settings);
    let approved = success(
        action(
            &host,
            &token,
            json!({"kind":"download-package","revision":first.digest}),
        )
        .await?,
    )
    .await?;
    assert_eq!(approved["data"]["revision"], first.digest);
    let response = host.client.get(&download).send().await?;
    assert!(response.status().is_success());
    assert_eq!(
        response.headers()["content-type"],
        "application/vnd.aio.component+gzip"
    );
    assert_eq!(response.bytes().await?.as_ref(), archive);
    for invalid in [
        json!({"kind":"download-package","revision":"../../identity"}),
        json!({"kind":"download-package","revision":first.digest,"url":"https://outside.test"}),
        json!({"kind":"open-settings","page_id":"profile"}),
        json!({"kind":"install-tool","id":"missing-tool","version":"1.0.0"}),
        json!({"kind":"install-tool","id":"codex-model-sync","version":"latest"}),
        json!({"kind":"open-url","url":"https://outside.test"}),
    ] {
        assert!(!action(&host, &token, invalid).await?.status().is_success());
    }
    let tool = success(
        action(
            &host,
            &token,
            json!({"kind":"install-tool","id":"codex-model-sync","version":"0.4.1"}),
        )
        .await?,
    )
    .await?;
    assert_eq!(
        tool["data"]["url"],
        "aio://install/codex-model-sync?version=0.4.1"
    );
    assert!(
        !reqwest::Client::new()
            .get(&download)
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(
        !host
            .client
            .get(&download)
            .header("cookie", "session=member")
            .send()
            .await?
            .status()
            .is_success()
    );
    for flag in [&identity.revoked, &identity.expired] {
        flag.store(true, Ordering::SeqCst);
        assert!(
            !host
                .client
                .get(&download)
                .send()
                .await?
                .status()
                .is_success()
        );
        assert!(
            !action(&host, &token, settings.clone())
                .await?
                .status()
                .is_success()
        );
        flag.store(false, Ordering::SeqCst);
    }
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/disable"),
            Value::Null,
        )
        .await?,
    )
    .await?;
    assert!(
        !host
            .client
            .get(&download)
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(!action(&host, &token, settings).await?.status().is_success());
    // 停用工具契约后，管理组件不能通过动作接口绕过插件组合。
    host.reconfigure(selection(&identity, &probes, &[&first]))
        .await?;
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/enable"),
            Value::Null,
        )
        .await?,
    )
    .await?;
    let token = host.mount(source).await?;
    assert!(
        !action(
            &host,
            &token,
            json!({"kind":"install-tool","id":"codex-model-sync","version":"0.4.1"})
        )
        .await?
        .status()
        .is_success()
    );
    host.stop().await
}
