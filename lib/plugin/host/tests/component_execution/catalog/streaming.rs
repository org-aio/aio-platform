use super::*;
use az_plugin_host::identity::{IdentityProvider, SessionContext};
use futures_util::StreamExt;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
struct ResourceIdentity {
    revoked: AtomicBool,
}
#[async_trait::async_trait]
impl IdentityProvider for ResourceIdentity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<SessionContext>> {
        Identity("tenant".into()).authenticate(headers).await
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Identity("tenant".into()).member_active(tenant, user).await
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Identity("tenant".into())
            .session_active(session, tenant, user)
            .await
    }
}
#[async_trait::async_trait]
impl az_plugin_host::authorization::AuthorizationProvider for ResourceIdentity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(true)
    }
    async fn session_has_permissions(
        &self,
        session: &str,
        tenant: &str,
        user: &str,
        permissions: &[String],
    ) -> Result<bool> {
        Ok(!self.revoked.load(Ordering::SeqCst)
            && self.session_active(session, tenant, user).await?
            && permissions.iter().all(|p| p == "plugin:manage"))
    }
}

fn resource_package(
    root: &std::path::Path,
    git: &str,
    version: &str,
) -> Result<az_plugin_bundle::Bundle> {
    let seed = package(root, git, 1, version)?;
    let path = root.join(version);
    let declaration = "schema_version=2\n[plugin]\nrequired_permissions=['plugin:manage']\n[[plugin.public_routes]]\npath='/i/{token}'\ncontent_types=['image/png']\nmax_bytes=104857600\n";
    std::fs::write(
        path.join("aio-plugin.toml"),
        seed.manifest.replace("schema_version=2\n", declaration),
    )?;
    az_plugin_bundle::Bundle::from_directory(
        &path,
        "aio-plugin.toml",
        git.into(),
        seed.commit,
        version.into(),
    )
}

#[tokio::test]
async fn public_and_private_resources_stream_with_installation_and_mount_authority() -> Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = FileCatalog::open(&root.path().join("catalog.json"));
    let probes: [Arc<Probe>; 2] = Default::default();
    let bundle = resource_package(
        root.path(),
        "https://github.com/example/resources.git",
        "1.0.0",
    )?;
    let identity = Arc::new(ResourceIdentity::default());
    let host = Host::start_selected(
        root.path(),
        "invalid database",
        selected_with_identity(
            root.path(),
            catalog.clone(),
            probes.clone(),
            true,
            identity.clone(),
        ),
    )
    .await?;
    let published = success(host.publish(&bundle).await?).await?;
    let source: Uuid = published["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .parse()?;
    success(host.install(&bundle).await?).await?;
    let token = host.mount(source).await?;
    let public = format!(
        "{}/api/runtime/components/public/{source}/tenant",
        host.base
    );
    let download = format!("{}/api/runtime/components/{token}/download", host.base);
    let anonymous = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = anonymous
        .get(format!("{public}/i/one"))
        .header("cookie", "forged")
        .header("x-aio-user-id", "admin")
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert!(!response.headers().contains_key("set-cookie"));
    assert!(!response.headers().contains_key("location"));
    assert!(
        response.headers()["content-security-policy"]
            .to_str()?
            .contains("sandbox")
    );
    assert_eq!(response.bytes().await?.as_ref(), &[0x5a; 64]);
    assert!(probes[0].resource_calls.lock().await[0].1.is_none());
    for path in [
        "/private/one",
        "/i/one/extra",
        "/i/html",
        "/i/duplicate",
        "/i/redirect",
    ] {
        assert!(
            !anonymous
                .get(format!("{public}{path}"))
                .send()
                .await?
                .status()
                .is_success(),
            "{path}"
        );
    }
    assert_eq!(
        anonymous
            .post(format!("{public}/i/one"))
            .send()
            .await?
            .status(),
        405
    );
    assert!(
        !anonymous
            .get(public.replace("/tenant", "/other") + "/i/one")
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(
        !anonymous
            .get(&download)
            .query(&[("path", "/large"), ("name", "file.bin")])
            .send()
            .await?
            .status()
            .is_success()
    );
    for path in ["//outside", "/a/../b", "/a/%2f"] {
        assert!(
            !host
                .client
                .get(&download)
                .query(&[("path", path), ("name", "file.bin")])
                .send()
                .await?
                .status()
                .is_success()
        );
    }
    let response = host
        .client
        .get(&download)
        .query(&[("path", "/large"), ("name", "资料.bin")])
        .send()
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert!(
        response.headers()["content-disposition"]
            .to_str()?
            .contains("filename*=UTF-8''")
    );
    let mut stream = response.bytes_stream();
    let mut total = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        assert!(chunk.iter().all(|byte| *byte == 0x5a));
        total += chunk.len();
    }
    assert_eq!(total, 100 * 1024 * 1024);
    let calls = probes[0].resource_calls.lock().await;
    assert!(calls.last().context("调用缺失")?.1.is_some());
    drop(calls);
    let mut stalled = Vec::new();
    for _ in 0..16 {
        let response = anonymous.get(format!("{public}/i/slow")).send().await?;
        assert_eq!(response.status(), 200);
        stalled.push(response);
    }
    assert_eq!(
        anonymous
            .get(format!("{public}/i/one"))
            .send()
            .await?
            .status(),
        429
    );
    drop(stalled);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let response = anonymous.get(format!("{public}/i/one")).send().await?;
            if response.status() == 200 {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await??;
    let response = anonymous.get(format!("{public}/i/short")).send().await?;
    assert!(response.bytes().await.is_err());
    let mut private = host
        .client
        .get(&download)
        .query(&[("path", "/slow"), ("name", "file.bin")])
        .send()
        .await?
        .bytes_stream();
    assert!(private.next().await.context("缺少首帧")?.is_ok());
    host.client
        .delete(format!("{}/api/runtime/frontend/{token}", host.base))
        .send()
        .await?
        .error_for_status()?;
    let revoked = tokio::time::timeout(std::time::Duration::from_secs(3), private.next()).await?;
    assert!(revoked.is_none_or(|chunk| chunk.is_err()));
    let renewed = host.mount(source).await?;
    let mut permission_stream = host
        .client
        .get(format!(
            "{}/api/runtime/components/{renewed}/download",
            host.base
        ))
        .query(&[("path", "/slow"), ("name", "file.bin")])
        .send()
        .await?
        .bytes_stream();
    assert!(permission_stream.next().await.context("缺少首帧")?.is_ok());
    identity.revoked.store(true, Ordering::SeqCst);
    let revoked =
        tokio::time::timeout(std::time::Duration::from_secs(3), permission_stream.next()).await?;
    assert!(revoked.is_none_or(|chunk| chunk.is_err()));
    assert_eq!(
        anonymous
            .get(format!("{public}/i/one"))
            .send()
            .await?
            .status(),
        200
    );
    identity.revoked.store(false, Ordering::SeqCst);
    let mut shared = anonymous
        .get(format!("{public}/i/slow"))
        .send()
        .await?
        .bytes_stream();
    assert!(shared.next().await.context("缺少首帧")?.is_ok());
    success(
        host.post(&format!("/api/runtime/plugins/{source}/disable"), json!({}))
            .await?,
    )
    .await?;
    let revoked = tokio::time::timeout(std::time::Duration::from_secs(3), shared.next()).await?;
    assert!(revoked.is_none_or(|chunk| chunk.is_err()));
    assert!(
        !anonymous
            .get(format!("{public}/i/one"))
            .send()
            .await?
            .status()
            .is_success()
    );
    host.stop().await
}
