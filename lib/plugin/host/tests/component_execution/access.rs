//! 真实 HTTP 与替代执行器验证整包门槛，插件安装不能扩大宿主管理权限。
use anyhow::{Context, Result};
use axum::http::HeaderMap;
use az_plugin_bundle::Bundle;
use az_plugin_host::{
    authorization::AuthorizationProvider,
    identity::{IdentityProvider, SessionContext},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

use super::fixture::{self, Host, Probe};

struct Identity {
    tenant: String,
    revoked: AtomicBool,
    second_permission: AtomicBool,
    expired: AtomicBool,
    removed: AtomicBool,
    unavailable: AtomicBool,
}

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let user = match headers.get("cookie").and_then(|value| value.to_str().ok()) {
            Some("session=component-test") => "admin",
            Some("session=member") => "member",
            Some("session=other") => "other",
            _ => return Ok(None),
        };
        Ok(Some(SessionContext {
            session_id: format!("session:{user}"),
            user_id: user.into(),
            tenant_id: if user == "other" {
                "other-tenant".into()
            } else {
                self.tenant.clone()
            },
            account: user.into(),
            display_name: user.into(),
            tenant_label: "Test".into(),
            // 故意保留旧快照；撤权必须通过实时提供者生效。
            permissions: if user == "admin" {
                vec!["plugin:manage".into(), "dictionary:manage".into()]
            } else {
                vec![]
            },
        }))
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(!self.removed.load(Ordering::SeqCst)
            && ((tenant == self.tenant && matches!(user, "admin" | "member"))
                || (tenant == "other-tenant" && user == "other")))
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(!self.expired.load(Ordering::SeqCst)
            && session == format!("session:{user}")
            && self.member_active(tenant, user).await?)
    }
}

#[async_trait::async_trait]
impl AuthorizationProvider for Identity {
    async fn can_publish(&self, session: &SessionContext) -> Result<bool> {
        Ok(session.user_id == "admin")
    }
    async fn session_has_permissions(
        &self,
        session: &str,
        tenant: &str,
        user: &str,
        permissions: &[String],
    ) -> Result<bool> {
        anyhow::ensure!(
            !self.unavailable.load(Ordering::SeqCst),
            "测试授权提供者不可用"
        );
        Ok(user == "admin"
            && !self.revoked.load(Ordering::SeqCst)
            && self.session_active(session, tenant, user).await?
            && permissions.iter().all(|permission| {
                permission == "dictionary:manage"
                    || (permission == "tenant:manage"
                        && self.second_permission.load(Ordering::SeqCst))
            }))
    }
}

fn protect(root: &std::path::Path, bundle: Bundle, permissions: &[&str]) -> Result<Bundle> {
    let required = serde_json::to_string(permissions)?;
    let manifest = bundle.manifest.replace("[plugin.marketplace]", &format!("[plugin]\npermissions=['manage']\nrequired_permissions={required}\n[plugin.marketplace]"));
    let root = root.join(&bundle.version);
    std::fs::write(root.join("aio-plugin.toml"), manifest)?;
    Bundle::from_directory(
        &root,
        "aio-plugin.toml",
        bundle.git,
        bundle.commit,
        bundle.version,
    )
}

async fn catalog(host: &Host, cookie: &str) -> Result<Value> {
    fixture::success(
        host.client
            .get(format!("{}/api/runtime/bootstrap", host.base))
            .header("cookie", cookie)
            .send()
            .await?,
    )
    .await
}

async fn unavailable(host: &Host, source: Uuid, token: &str, cookie: &str) -> Result<()> {
    let mount = host
        .client
        .post(format!("{}/api/runtime/frontend/mount", host.base))
        .header("cookie", cookie)
        .header("x-aio-user-id", "admin")
        .header("x-aio-permissions", "dictionary:manage")
        .json(&json!({"page_id":format!("component:{source}:home")}))
        .send()
        .await?;
    assert!(mount.status().is_client_error());
    for endpoint in ["request", "renew"] {
        let response = host
            .client
            .post(format!(
                "{}/api/runtime/components/{token}/{endpoint}",
                host.base
            ))
            .header("cookie", cookie)
            .json(&json!({"method":"GET","path":"/test"}))
            .send()
            .await?;
        assert!(response.status().is_client_error());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn protected_packages_check_live_host_rights_across_http_upgrade_and_rollback() -> Result<()>
{
    let database = fixture::database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/protected-{tenant}.git");
    let identity = Arc::new(Identity {
        tenant,
        revoked: false.into(),
        second_permission: false.into(),
        expired: false.into(),
        removed: false.into(),
        unavailable: false.into(),
    });
    let probes: [Arc<Probe>; 2] = Default::default();
    let host =
        Host::start_with_identity(root.path(), &database, identity.clone(), probes.clone()).await?;
    let first = protect(
        root.path(),
        fixture::package(root.path(), &git, 1, "1.0.0")?,
        &["dictionary:manage"],
    )?;
    let published = fixture::success(host.publish(&first).await?).await?;
    let source: Uuid = published["data"]["source_id"]
        .as_str()
        .context("缺少来源")?
        .parse()?;
    fixture::success(host.install(&first).await?).await?;
    let token = host.assert_call(source, 1, &first).await?;
    let member = catalog(&host, "session=member").await?;
    assert!(
        member["data"]["catalog"]["pages"]
            .as_array()
            .context("缺少目录")?
            .is_empty()
    );
    assert_eq!(member["data"]["catalog"]["plugins"][0]["state"], "active");
    let permissions = member["data"]["permissions"]
        .as_array()
        .context("缺少权限")?;
    assert!(permissions.contains(&json!(format!("component:{source}:manage"))));
    assert!(!permissions.contains(&json!("dictionary:manage")));
    assert!(!permissions.contains(&json!("plugin:manage")));
    unavailable(&host, source, &token, "session=member").await?;
    unavailable(&host, source, &token, "session=other").await?;
    let asset = format!(
        "{}/api/runtime/components/assets/{token}/index.html",
        host.base
    );
    host.client.get(&asset).send().await?.error_for_status()?;

    let calls = probes[0]
        .events
        .lock()
        .await
        .iter()
        .filter(|event| **event == "handle")
        .count();
    for flag in [
        &identity.revoked,
        &identity.expired,
        &identity.removed,
        &identity.unavailable,
    ] {
        flag.store(true, Ordering::SeqCst);
        unavailable(&host, source, &token, "session=component-test").await?;
        assert!(
            host.client
                .get(&asset)
                .send()
                .await?
                .status()
                .is_client_error()
        );
        flag.store(false, Ordering::SeqCst);
    }
    assert_eq!(
        probes[0]
            .events
            .lock()
            .await
            .iter()
            .filter(|event| **event == "handle")
            .count(),
        calls
    );
    identity.revoked.store(true, Ordering::SeqCst);
    let revoked = catalog(&host, "session=component-test").await?;
    assert!(
        revoked["data"]["catalog"]["pages"]
            .as_array()
            .context("缺少目录")?
            .is_empty()
    );
    identity.revoked.store(false, Ordering::SeqCst);
    host.assert_call(source, 1, &first).await?;

    let second = protect(
        root.path(),
        fixture::package(root.path(), &git, 2, "2.0.0")?,
        &["dictionary:manage", "tenant:manage"],
    )?;
    fixture::success(host.publish(&second).await?).await?;
    host.assert_call(source, 1, &first).await?;
    fixture::success(host.install(&second).await?).await?;
    unavailable(&host, source, &token, "session=component-test").await?;
    identity.second_permission.store(true, Ordering::SeqCst);
    let upgraded = host.assert_call(source, 2, &second).await?;
    assert!(!host.invoke(&token).await?.status().is_success());
    fixture::success(
        host.post(
            &format!("/api/runtime/plugins/{source}/rollback"),
            json!({}),
        )
        .await?,
    )
    .await?;
    identity.second_permission.store(false, Ordering::SeqCst);
    host.assert_call(source, 1, &first).await?;
    assert!(!host.invoke(&upgraded).await?.status().is_success());

    // 旧索引回填只消费整包，不把宿主权限写入安装权限。
    let pool = host.state.service::<sqlx::PgPool>()?;
    sqlx::query("UPDATE component_versions SET required_permissions=NULL WHERE digest=$1")
        .bind(&first.digest)
        .execute(pool.as_ref())
        .await?;
    host.stop().await?;
    let host = Host::start_with_identity(root.path(), &database, identity, probes).await?;
    host.assert_call(source, 1, &first).await?;
    let member = catalog(&host, "session=member").await?;
    assert!(
        member["data"]["catalog"]["pages"]
            .as_array()
            .context("缺少目录")?
            .is_empty()
    );
    unavailable(&host, source, &token, "session=member").await?;
    fixture::success(
        host.post(
            &format!("/api/runtime/plugins/{source}/uninstall"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.stop().await?;

    // 仅实现发布策略的既有提供者默认拒绝宿主权限，不回退到安装权限。
    let host = Host::start(root.path(), &database, "default-policy", Default::default()).await?;
    fixture::success(host.install(&first).await?).await?;
    assert!(host.mount(source).await.is_err());
    fixture::success(
        host.post(
            &format!("/api/runtime/plugins/{source}/uninstall"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.stop().await
}
