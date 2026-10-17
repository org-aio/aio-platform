//! 文档图片经真实管理挂载读取，中文路径与撤权语义和原正式处理器一致。

use super::*;
use base64::Engine;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn documentation_images_require_active_trusted_management_and_public_sources() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/manager-images-{tenant}.git");
    let identity = Arc::new(Identity {
        tenant,
        revoked: false.into(),
        expired: false.into(),
        publisher: true.into(),
    });
    let probes: [Arc<Probe>; 2] = Default::default();
    let bundle = manager(root.path(), &git, "1.0.0")?;
    let mut selected = selection(&identity, &probes, &[&bundle]);
    selected.push(Arc::new(DocumentationPlugin));
    selected.push(Arc::new(PostgresDocumentationPlugin));
    let mut host = Host::start_selected(root.path(), &database, selected).await?;
    let source: Uuid = success(host.publish(&bundle).await?).await?["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .parse()?;
    success(host.install(&bundle).await?).await?;
    let token = host.mount(source).await?;
    let pool = host.state.service::<sqlx::PgPool>()?;
    let document = Uuid::new_v4().to_string();
    let revision = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO plugin_sources(id,git) VALUES($1,$2)")
        .bind(&document)
        .bind(format!("https://example.com/{document}.git"))
        .execute(pool.as_ref())
        .await?;
    sqlx::query("INSERT INTO plugin_revisions(id,source_id,revision,runtime,manifest,pages) VALUES($1,$1,$2,'page-definition','{}','[]')").bind(&document).bind(&revision).execute(pool.as_ref()).await?;
    sqlx::query("INSERT INTO plugin_packages(revision,git,version,manifest_toml,artifact_sha256,archive) VALUES($1,$2,'1.0.0','',$1,$3)")
        .bind(&revision)
        .bind(format!("https://example.com/{document}.git"))
        .bind(Vec::<u8>::new())
        .execute(pool.as_ref())
        .await?;
    let bytes = b"\x89PNG\r\n\x1a\n\x00\xff\x80";
    let images = json!({"图片.png":{"content_type":"image/png","content_base64":base64::engine::general_purpose::STANDARD.encode(bytes)}});
    sqlx::query("INSERT INTO plugin_documents(revision,readme,images) VALUES($1,'', $2)")
        .bind(&revision)
        .bind(images)
        .execute(pool.as_ref())
        .await?;
    let url = format!("/api/runtime/marketplace/{revision}/images/%E5%9B%BE%E7%89%87.png");
    let endpoint = format!("/api/runtime/components/{token}/image");
    let response = host.post(&endpoint, json!({"url":url})).await?;
    assert!(response.status().is_success(), "{}", response.status());
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.bytes().await?.as_ref(), bytes);
    for invalid in [
        "/api/identity/me",
        "https://localhost/image.png",
        "https://127.0.0.1/image.png",
        "https://169.254.169.254/image.png",
        "https://user:password@example.com/image.png",
    ] {
        assert!(
            !host
                .post(&endpoint, json!({"url":invalid}))
                .await?
                .status()
                .is_success()
        );
    }
    for flag in [&identity.revoked, &identity.expired] {
        flag.store(true, Ordering::SeqCst);
        assert!(
            !host
                .post(&endpoint, json!({"url":url}))
                .await?
                .status()
                .is_success()
        );
        flag.store(false, Ordering::SeqCst);
    }
    host.reconfigure(selection(&identity, &probes, &[&bundle]))
        .await?;
    let token = host.mount(source).await?;
    assert_eq!(
        host.post(
            &format!("/api/runtime/components/{token}/image"),
            json!({"url":url})
        )
        .await?
        .status(),
        404
    );
    host.stop().await
}
