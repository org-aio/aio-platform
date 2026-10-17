//! 通过真实 HTTP、数据库与在线组合验证部署信任、交互授权和无执行器占用的自管理。

use super::{fixture::*, identity_support};
use anyhow::{Context, Result};
use az_plugin_bundle::Bundle;
use az_plugin_core::DynPlugin;
use az_plugin_host::{
    authorization::AuthorizationProvider,
    identity::{IdentityProvider, SessionContext},
    runtime::server::plugins::*,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[path = "management_actions.rs"]
mod actions;

#[path = "management_images.rs"]
mod images;

struct Identity {
    tenant: String,
    revoked: AtomicBool,
    expired: AtomicBool,
    publisher: AtomicBool,
}

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<SessionContext>> {
        let Some(cookie) = headers.get("cookie").and_then(|h| h.to_str().ok()) else {
            return Ok(None);
        };
        let user = if cookie == "session=component-test" {
            "manager"
        } else {
            "member"
        };
        Ok(Some(SessionContext {
            session_id: "session".into(),
            user_id: user.into(),
            account: user.into(),
            display_name: user.into(),
            tenant_id: self.tenant.clone(),
            tenant_label: "Tenant".into(),
            // 故意保留旧快照，正式入口仍须检查实时权限。
            permissions: if user == "manager" {
                vec!["plugin:manage".into()]
            } else {
                vec![]
            },
        }))
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == self.tenant && matches!(user, "manager" | "member"))
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "session"
            && !self.expired.load(Ordering::SeqCst)
            && self.member_active(tenant, user).await?)
    }
}

#[async_trait::async_trait]
impl AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(self.publisher.load(Ordering::SeqCst))
    }
    async fn session_has_permissions(
        &self,
        session: &str,
        tenant: &str,
        user: &str,
        required: &[String],
    ) -> Result<bool> {
        Ok(user == "manager"
            && !self.revoked.load(Ordering::SeqCst)
            && self.session_active(session, tenant, user).await?
            && required == ["plugin:manage"])
    }
}

fn manager(root: &Path, git: &str, version: &str) -> Result<Bundle> {
    let bundle = package(root, git, 1, version)?;
    let manifest = bundle.manifest.replace("[plugin.marketplace]", "[plugin]\nrequired_permissions=['plugin:manage']\n[plugin.capabilities]\nmanagement=true\n[plugin.marketplace]");
    let root = root.join(version);
    std::fs::write(root.join("aio-plugin.toml"), manifest)?;
    Bundle::from_directory(
        &root,
        "aio-plugin.toml",
        bundle.git,
        bundle.commit,
        bundle.version,
    )
}

fn selection(
    identity: &Arc<Identity>,
    probes: &[Arc<Probe>; 2],
    bundles: &[&Bundle],
) -> Vec<DynPlugin<RuntimeAssembly>> {
    let mut selected = plugins(probes.clone(), true);
    if !bundles.is_empty() {
        selected.push(Arc::new(ComponentManagementPlugin(
            bundles
                .iter()
                .map(|bundle| TrustedManagementComponent {
                    git: bundle.git.clone(),
                    digest: bundle.digest.clone(),
                })
                .collect(),
        )));
    }
    identity_support::with_identity(identity.clone(), selected)
}

async fn manage(
    host: &Host,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> Result<reqwest::Response> {
    host.post(&format!("/api/runtime/components/{token}/manage"), json!({
        "method":method,"path":path,"headers":[{"name":"content-type","value":"application/json"}],
        "body":if body.is_null() { Vec::new() } else { serde_json::to_vec(&body)? }
    })).await
}

async fn output(response: reqwest::Response) -> Result<Value> {
    let value = success(response).await?;
    assert_eq!(value["data"]["status"], 200, "{value}");
    let body: Vec<u8> = serde_json::from_value(value["data"]["body"].clone())?;
    Ok(serde_json::from_slice(&body)?)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn management_is_version_trusted_and_frontend_can_manage_itself() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/manager-{tenant}.git");
    let identity = Arc::new(Identity {
        tenant: tenant.clone(),
        revoked: false.into(),
        expired: false.into(),
        publisher: true.into(),
    });
    let probes: [Arc<Probe>; 2] = Default::default();
    let first = manager(root.path(), &git, "1.0.0")?;
    let second = manager(root.path(), &git, "2.0.0")?;
    let selected = selection(&identity, &probes, &[]);
    let mut host = Host::start_selected(root.path(), &database, selected).await?;
    assert!(!host.publish(&first).await?.status().is_success());
    host.reconfigure(selection(&identity, &probes, &[&first]))
        .await?;
    let source: Uuid = success(host.publish(&first).await?).await?["data"]["source_id"]
        .as_str()
        .context("缺少来源")?
        .parse()?;
    success(host.install(&first).await?).await?;
    let token = host.mount(source).await?;
    let catalog =
        output(manage(&host, &token, "GET", "/api/runtime/catalog", Value::Null).await?).await?;
    assert_eq!(catalog["data"]["tenant"]["id"], tenant);
    // 替代执行器通过同一 HostServices 契约获取管理目录，没有 cookie 或身份头。
    output(
        host.post(
            &format!("/api/runtime/components/{token}/request"),
            json!({"method":"GET","path":"/api/runtime/catalog"}),
        )
        .await?,
    )
    .await?;
    assert!(
        !host
            .post(
                &format!("/api/runtime/components/{token}/request"),
                json!({"method":"POST","path":format!("/api/runtime/plugins/{source}/disable")})
            )
            .await?
            .status()
            .is_success()
    );
    for name in ["cookie", "authorization", "x-aio-user-id"] {
        let response = host.post(&format!("/api/runtime/components/{token}/manage"), json!({"method":"GET","path":"/api/runtime/catalog","headers":[{"name":name,"value":"forged"}]})).await?;
        assert!(!response.status().is_success());
    }
    let member = host
        .client
        .post(format!(
            "{}/api/runtime/components/{token}/manage",
            host.base
        ))
        .header("cookie", "session=member")
        .json(&json!({"method":"GET","path":"/api/runtime/catalog"}))
        .send()
        .await?;
    assert!(!member.status().is_success());
    for flag in [&identity.revoked, &identity.expired] {
        flag.store(true, Ordering::SeqCst);
        assert!(
            !manage(&host, &token, "GET", "/api/runtime/catalog", Value::Null)
                .await?
                .status()
                .is_success()
        );
        flag.store(false, Ordering::SeqCst);
    }
    identity.publisher.store(false, Ordering::SeqCst);
    let forbidden = success(
        manage(
            &host,
            &token,
            "DELETE",
            "/api/runtime/marketplace",
            json!({"git":git}),
        )
        .await?,
    )
    .await?;
    assert_eq!(forbidden["data"]["status"], 403);
    identity.publisher.store(true, Ordering::SeqCst);
    assert!(!host.publish(&second).await?.status().is_success());
    let old_runtime = host.state.clone();
    let old_resources = probes[0]
        .context
        .lock()
        .await
        .clone()
        .context("执行环境缺失")?
        .resources(source, &tenant, &first.verify()?)
        .await?;
    host.reconfigure(selection(&identity, &probes, &[&first, &second]))
        .await?;
    success(host.publish(&second).await?).await?;
    let token = host.mount(source).await?;
    {
        // 执行器已切到新版而安装记录尚未提交时，新的浏览器挂载必须等待。
        probes[0].pause_activate.store(true, Ordering::SeqCst);
        let upgrade = manage(
            &host,
            &token,
            "POST",
            "/api/runtime/plugins/install",
            json!({"git":git,"rev":second.digest}),
        );
        tokio::pin!(upgrade);
        tokio::select! {
            _ = probes[0].activation_entered.notified() => {},
            result = &mut upgrade => anyhow::bail!("升级未等待测试提交点: {:?}", result?.status()),
            _ = tokio::time::sleep(Duration::from_secs(5)) => anyhow::bail!("升级未进入测试提交点"),
        }
        let mounting = host.post(
            "/api/runtime/frontend/mount",
            json!({"page_id":format!("component:{source}:home")}),
        );
        tokio::pin!(mounting);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut mounting)
                .await
                .is_err()
        );
        probes[0].pause_activate.store(false, Ordering::SeqCst);
        probes[0].activation_resume.notify_one();
        output(tokio::time::timeout(Duration::from_secs(5), &mut upgrade).await??).await?;
        let mounted =
            success(tokio::time::timeout(Duration::from_secs(5), &mut mounting).await??).await?;
        assert_eq!(mounted["data"]["revision"], second.digest);
    }
    assert!(
        !manage(&host, &token, "GET", "/api/runtime/catalog", Value::Null)
            .await?
            .status()
            .is_success()
    );
    let token = host.mount(source).await?;
    output(
        tokio::time::timeout(
            Duration::from_secs(5),
            manage(
                &host,
                &token,
                "POST",
                &format!("/api/runtime/plugins/{source}/rollback"),
                Value::Null,
            ),
        )
        .await??,
    )
    .await?;
    let token = host.mount(source).await?;
    let before: (String, Uuid, Option<String>) = sqlx::query_as("SELECT digest,generation,excluded_digest FROM component_installations WHERE source_id=$1 AND tenant_id=$2").bind(source).bind(&tenant).fetch_one(host.state.service::<sqlx::PgPool>()?.as_ref()).await?;
    host.reconfigure(selection(&identity, &probes, &[])).await?;
    let after: (String, Uuid, Option<String>) = sqlx::query_as("SELECT digest,generation,excluded_digest FROM component_installations WHERE source_id=$1 AND tenant_id=$2").bind(source).bind(&tenant).fetch_one(host.state.service::<sqlx::PgPool>()?.as_ref()).await?;
    assert_eq!(before, after);
    assert!(
        !manage(&host, &token, "GET", "/api/runtime/catalog", Value::Null)
            .await?
            .status()
            .is_success()
    );
    let rejected = old_resources
        .services
        .context("宿主服务缺失")?
        .manage(
            &az_plugin_contract::InvocationScope {
                source_id: source.to_string(),
                revision: first.digest.clone(),
                context: Default::default(),
                grants: first.verify()?.manifest().plugin.capabilities.clone(),
            },
            az_plugin_contract::transport::Request {
                method: "GET".into(),
                path: "/api/runtime/catalog".into(),
                query: None,
                headers: vec![],
                body: vec![],
            },
        )
        .await;
    assert!(rejected.is_err());
    drop(old_runtime);
    host.reconfigure(selection(&identity, &probes, &[&first]))
        .await?;
    let token = host.mount(source).await?;
    output(
        tokio::time::timeout(
            Duration::from_secs(5),
            manage(
                &host,
                &token,
                "POST",
                &format!("/api/runtime/plugins/{source}/disable"),
                Value::Null,
            ),
        )
        .await??,
    )
    .await?;
    host.stop().await?;
    let host = Host::start_selected(
        root.path(),
        &database,
        selection(&identity, &probes, &[&first]),
    )
    .await?;
    assert!(host.mount(source).await.is_err());
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/enable"),
            Value::Null,
        )
        .await?,
    )
    .await?;
    let token = host.mount(source).await?;
    output(
        tokio::time::timeout(
            Duration::from_secs(5),
            manage(
                &host,
                &token,
                "POST",
                &format!("/api/runtime/plugins/{source}/uninstall"),
                Value::Null,
            ),
        )
        .await??,
    )
    .await?;
    host.stop().await
}
