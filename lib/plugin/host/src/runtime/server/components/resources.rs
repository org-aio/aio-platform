use std::{panic::AssertUnwindSafe, path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_runtime::{DatabaseProvisioner, InvocationResources, Keyring, ObjectStore};
use futures_util::FutureExt;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    Components,
    execution::{ComponentExecutors, ExecutionContext},
    permissions, services,
};

impl Components {
    pub async fn open(
        pool: PgPool,
        storage: &crate::configuration::ComponentStorage,
        keyring: Arc<Keyring>,
        #[cfg(feature = "sidecar")] identity: Arc<dyn crate::identity::IdentityProvider>,
        runtime_services: dill::CatalogWeakRef,
        enhancements: Arc<super::super::enhancements::Enhancements>,
        executors: Arc<ComponentExecutors>,
    ) -> Result<Arc<Self>> {
        executors.validate()?;
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await?;
        permissions::backfill(&pool).await?;
        let provisioner = DatabaseProvisioner::connect(&storage.database_url).await?;
        let services = Arc::new(services::Services::with_enhancements(enhancements.clone()));
        let components = Arc::new(Self {
            #[cfg(feature = "sidecar")]
            enhancements,
            pool,
            development: Default::default(),
            provisioner,
            keyring,
            objects: storage.root.join("objects"),
            services,
            #[cfg(feature = "sidecar")]
            identity,
            runtime_services,
            executors,
            active: Default::default(),
            mutations: Default::default(),
        });
        let context = ExecutionContext {
            root: storage.root.clone(),
            components: Arc::downgrade(&components),
        };
        // 原生实现可能在部分初始化后 panic，此时资源尚未交给外层生命周期。
        let result = AssertUnwindSafe(components.executors.initialize(context))
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("整包执行器初始化时 panic")));
        if let Err(error) = result {
            let cleanup = components.shutdown().await;
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("执行器初始化失败且补偿失败: {cleanup:#}")))
                }
            };
        }
        Ok(components)
    }

    pub(super) async fn resources_verified(
        &self,
        source: Uuid,
        tenant: &str,
        verified: &VerifiedBundle,
    ) -> Result<InvocationResources> {
        let grants = &verified.manifest().plugin.capabilities;
        ensure!(
            !grants.management && !grants.identity_provider,
            "当前发布入口未开放宿主管理或身份提供能力"
        );
        let migrations = verified
            .migrations()
            .map(|(name, sql)| (name.to_owned(), sql.to_owned()))
            .collect::<Vec<_>>();
        let resources = InvocationResources {
            database: if grants.database {
                Some(
                    self.provisioner
                        .install(&source.to_string(), tenant, &migrations, &self.keyring)
                        .await?,
                )
            } else {
                None
            },
            storage: if grants.storage {
                Some(ObjectStore::open(self.objects.clone(), &source.to_string(), tenant).await?)
            } else {
                None
            },
            keyring: Some(self.keyring.clone()),
            services: Some(self.services.clone()),
        };
        resources.validate_scope(&source.to_string(), tenant)?;
        Ok(resources)
    }
}

pub(in crate::runtime::server) fn load_keyring(path: &Path) -> Result<Keyring> {
    use std::{
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    if !path.exists() {
        std::fs::create_dir_all(path.parent().context("密钥目录无效")?)?;
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("生成主密钥失败: {e}"))?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(mut file) => file.write_all(&serde_json::to_vec(&key)?)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    ensure!(
        std::fs::metadata(path)?.permissions().mode() & 0o077 == 0,
        "宿主主密钥文件权限必须为 0600"
    );
    let key: [u8; 32] = serde_json::from_slice(&std::fs::read(path)?)?;
    Keyring::new("primary".into(), [("primary".into(), key)].into())
}
