//! 使用隔离 PostgreSQL schema 验证业务插件的建表归属、真实读写及重启恢复。

use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_runtime::Keyring;
use base64::Engine;
use sqlx::{PgPool, postgres::PgConnectOptions};

use super::*;
use crate::{
    configuration::HostConfig,
    generated::{clipboard, personal_config, worker, worker_webview},
    identity::SessionContext,
};

#[derive(Clone)]
struct Storage {
    options: PgConnectOptions,
}

#[async_trait::async_trait]
impl DatabaseProvider for Storage {
    async fn connect(&self, _: &HostConfig) -> Result<PgPool> {
        Ok(PgPool::connect_with(self.options.clone()).await?)
    }
}

#[async_trait::async_trait]
impl KeyringProvider for Storage {
    async fn load(&self, _: &HostConfig) -> Result<Keyring> {
        Keyring::new("test".into(), BTreeMap::from([("test".into(), [17; 32])]))
    }
}

impl Plugin<RuntimeAssembly> for Storage {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(self.clone())
                .bind::<dyn DatabaseProvider, Self>()
                .bind::<dyn KeyringProvider, Self>();
            Ok(())
        })
    }
}

struct Fixture {
    storage: Storage,
    admin: PgPool,
    schema: String,
    root: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let url = std::env::var("AIO_SERVICE_TEST_DATABASE_URL")?;
        let parsed = reqwest::Url::parse(&url)?;
        ensure!(
            parsed.host_str() == Some("127.0.0.1") && parsed.path().ends_with("_test"),
            "只接受本机独立测试库"
        );
        let admin = PgPool::connect(&url).await?;
        let schema = format!("services_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await?;
        let options = url
            .parse::<PgConnectOptions>()?
            .options([("search_path", schema.as_str())]);
        Ok(Self {
            storage: Storage { options },
            admin,
            schema,
            root: tempfile::tempdir()?,
        })
    }

    async fn start(&self, mut plugins: Vec<DynPlugin<RuntimeAssembly>>) -> Result<RuntimeState> {
        plugins.push(Arc::new(self.storage.clone()));
        plugins.push(Arc::new(DatabasePlugin));
        RuntimeState::initialize(tests::config(self.root.path()), plugins).await
    }

    async fn contains(&self, table: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
                .bind(format!("{}.{}", self.schema, table))
                .fetch_one(&self.admin)
                .await?,
        )
    }

    async fn finish(self) -> Result<()> {
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await?;
        self.admin.close().await;
        Ok(())
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn catalog_storage_does_not_create_unselected_business_tables() -> Result<()> {
    let fixture = Fixture::new().await?;
    let state = fixture.start(vec![Arc::new(StorePlugin)]).await?;
    assert!(fixture.contains("plugin_sources").await?);
    for table in [
        "worker_devices",
        "worker_webview_sessions",
        "personal_config_heads",
        "clipboard_heads",
    ] {
        assert!(
            !fixture.contains(table).await?,
            "未选择的业务表被创建: {table}"
        );
    }
    state.shutdown().await?;
    fixture.finish().await
}

fn business_plugins() -> Vec<DynPlugin<RuntimeAssembly>> {
    // 故意把依赖放到末尾，证明迁移顺序来自插件依赖而不是列表顺序。
    vec![
        Arc::new(ClipboardServicePlugin),
        Arc::new(PersonalConfigServicePlugin),
        Arc::new(WorkerWebviewServicePlugin),
        Arc::new(KeyringPlugin),
        Arc::new(WorkerServicePlugin),
    ]
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn business_services_persist_and_restart_without_plugin_catalog() -> Result<()> {
    let fixture = Fixture::new().await?;
    let outcome = async {
        let state = fixture.start(vec![Arc::new(WorkerServicePlugin)]).await?;
        assert!(fixture.contains("worker_devices").await?);
        for table in [
            "worker_webview_sessions",
            "personal_config_heads",
            "clipboard_heads",
            "plugin_sources",
        ] {
            assert!(!fixture.contains(table).await?);
        }
        let session = SessionContext {
            session_id: "session".into(),
            tenant_id: "tenant".into(),
            user_id: "owner".into(),
            account: "owner".into(),
            display_name: "Owner".into(),
            tenant_label: "Tenant".into(),
            permissions: vec![],
        };
        let workers = state.service::<dyn worker::WorkerService>()?;
        let pairing = workers
            .pair(worker::model::PairRequest {
                label: "Paired device".into(),
                platform: "linux".into(),
                capabilities: vec!["clipboard.sync".into()],
                machine_id: None,
            })
            .await?;
        workers.approve(&session, &pairing.code).await?;
        let device = workers.identity(&pairing.token).await?;
        workers.heartbeat(&device, None).await?;
        assert_eq!(workers.list(&session).await?.len(), 1);
        let old_pool = state.service::<PgPool>()?;
        state.shutdown().await?;
        assert!(old_pool.is_closed());

        let state = fixture.start(business_plugins()).await?;
        assert!(state.store().is_err());
        for table in [
            "plugin_sources",
            "plugin_revisions",
            "marketplace_entries",
            "component_sources",
            "delivery_jobs",
        ] {
            assert!(
                !fixture.contains(table).await?,
                "业务服务仍隐式依赖目录: {table}"
            );
        }
        let workers = state.service::<dyn worker::WorkerService>()?;
        assert_eq!(workers.identity(&pairing.token).await?.id, device.id);
        let clips = state.service::<dyn clipboard::ClipboardService>()?;
        let owner = clipboard::model::Owner {
            tenant: session.tenant_id.clone(),
            user: session.user_id.clone(),
            device: Some(device.id.clone()),
        };
        clips.access(&owner, &device.id, true).await?;
        assert!(clips.devices(&owner).await?[0].enabled);
        let data = base64::engine::general_purpose::STANDARD.encode(b"persistent clipboard");
        let item = clips
            .write(
                &owner,
                clipboard::model::ClipWrite {
                    kind: "text".into(),
                    mime: "text/plain".into(),
                    name: None,
                    data: data.clone(),
                },
            )
            .await?;
        assert_eq!(clips.read(&owner, &item.id).await?.data, data);
        let other = clipboard::model::Owner {
            tenant: "other".into(),
            ..owner.clone()
        };
        assert!(clips.read(&other, &item.id).await.is_err());

        let configs = state.service::<dyn personal_config::PersonalConfigService>()?;
        let config_owner = personal_config::model::Owner {
            tenant: owner.tenant.clone(),
            user: owner.user.clone(),
            device: owner.device.clone(),
        };
        let entry = configs
            .write(
                &config_owner,
                personal_config::model::WriteEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    expected: None,
                    kind: "file".into(),
                    target: ".config/aio/example.txt".into(),
                    layer: "shared".into(),
                    format: "text".into(),
                    secret: true,
                    executable: false,
                    deleted: false,
                    content: "persistent config".into(),
                },
            )
            .await?;
        configs
            .report(&config_owner, serde_json::json!({"ready": true}))
            .await?;
        assert_eq!(configs.catalog(&config_owner).await?.devices.len(), 1);
        state
            .service::<dyn worker_webview::WorkerWebviewService>()?
            .expire(&device.id)
            .await?;
        assert!(fixture.contains("worker_webview_sessions").await?);
        state.shutdown().await?;

        // 新连接池和新服务实例重跑幂等迁移，已配对凭据与加密内容必须可继续使用。
        let state = fixture.start(business_plugins()).await?;
        assert_eq!(
            state
                .service::<dyn clipboard::ClipboardService>()?
                .read(&owner, &item.id)
                .await?
                .data,
            data
        );
        assert_eq!(
            state
                .service::<dyn personal_config::PersonalConfigService>()?
                .read(&config_owner, &entry.id, None)
                .await?
                .content,
            "persistent config"
        );
        let workers = state.service::<dyn worker::WorkerService>()?;
        assert_eq!(workers.identity(&pairing.token).await?.id, device.id);
        workers.revoke(&session, &device.id).await?;
        assert!(workers.identity(&pairing.token).await.is_err());
        state.shutdown().await?;
        Ok::<_, crate::runtime::server::http_error::RuntimeError>(())
    }
    .await;
    fixture.finish().await?;
    if let Err(error) = outcome {
        use axum::response::IntoResponse;
        let response = error.into_response();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
        anyhow::bail!("业务服务验收失败: {}", String::from_utf8_lossy(&body));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn failed_business_migration_rolls_back_its_schema() -> Result<()> {
    let fixture = Fixture::new().await?;
    sqlx::query(&format!(
        "CREATE TABLE {}.worker_tasks (incompatible TEXT)",
        fixture.schema
    ))
    .execute(&fixture.admin)
    .await?;
    let error = fixture
        .start(vec![Arc::new(WorkerServicePlugin)])
        .await
        .err()
        .context("不兼容结构必须拒绝启动")?;
    assert!(format!("{error:#}").contains("初始化业务插件持久化结构失败"));
    assert!(
        !fixture.contains("worker_devices").await?,
        "失败的迁移留下了部分表"
    );
    assert!(fixture.contains("worker_tasks").await?);
    sqlx::query(&format!("DROP TABLE {}.worker_tasks", fixture.schema))
        .execute(&fixture.admin)
        .await?;
    let state = fixture.start(vec![Arc::new(WorkerServicePlugin)]).await?;
    assert!(fixture.contains("worker_devices").await?);
    state.shutdown().await?;
    fixture.finish().await
}
