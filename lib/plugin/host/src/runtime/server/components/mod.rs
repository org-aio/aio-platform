mod catalog;
mod controller;
mod development;
mod frontend;
mod model;
pub(in crate::runtime::server) mod process;
mod process_lifecycle;
mod services;
mod store;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_runtime::{
    ComponentEngine, ComponentSlot, DatabaseProvisioner, InvocationResources, Keyring, ObjectStore,
    PersistentComponentSlot,
};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub(super) use controller::router;
pub(super) use frontend::mount;

pub(super) struct Components {
    pub pool: PgPool,
    development: tokio::sync::RwLock<std::collections::BTreeMap<Uuid, development::Instance>>,
    provisioner: DatabaseProvisioner,
    engine: ComponentEngine,
    keyring: Arc<Keyring>,
    objects: PathBuf,
    services: Arc<services::Services>,
    identity: Arc<dyn crate::identity::IdentityProvider>,
    slots: Mutex<HashMap<(Uuid, String), Arc<PersistentComponentSlot>>>,
    pub mutations: Mutex<()>,
    processes: process::Processes,
}

impl Components {
    pub async fn open(
        pool: PgPool,
        database: &str,
        key_path: &Path,
        objects: PathBuf,
        identity: Arc<dyn crate::identity::IdentityProvider>,
    ) -> Result<Arc<Self>> {
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await?;
        let provisioner = DatabaseProvisioner::connect(database).await?;
        let keyring = Arc::new(load_keyring(key_path)?);
        let engine = ComponentEngine::with_cache(
            &key_path.parent().unwrap_or(Path::new(".")).join("compiled"),
        )?;
        let root = std::env::var_os("AIO_PROCESS_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                key_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("processes")
            });
        let supervisor = process::Processes::client()?;
        Ok(Arc::new_cyclic(|weak| Self {
            pool,
            development: Default::default(),
            provisioner,
            engine,
            keyring,
            objects,
            services: Arc::default(),
            identity,
            slots: Mutex::default(),
            mutations: Mutex::new(()),
            processes: process::Processes::new(weak.clone(), root, supervisor),
        }))
    }

    async fn resources(
        &self,
        source: Uuid,
        tenant: &str,
        bundle: &Bundle,
    ) -> Result<InvocationResources> {
        self.resources_verified(source, tenant, &bundle.verify()?)
            .await
    }

    async fn resources_verified(
        &self,
        source: Uuid,
        tenant: &str,
        verified: &az_plugin_bundle::VerifiedBundle,
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
        Ok(InvocationResources {
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
        })
    }

    async fn slot(&self, source: Uuid, tenant: &str) -> Result<Arc<PersistentComponentSlot>> {
        let mut slots = self.slots.lock().await;
        let key = (source, tenant.to_owned());
        if let Some(slot) = slots.get(&key) {
            return Ok(slot.clone());
        }
        let slot = Arc::new(
            self.provisioner
                .component_slot(
                    source,
                    tenant.into(),
                    semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
                )
                .await?,
        );
        if let Some(stored) = slot.stored().await? {
            let resources = self.resources(source, tenant, &stored.bundle).await?;
            slot.restore(&self.engine, stored.grants, resources).await?;
        }
        slots.insert(key, slot.clone());
        Ok(slot)
    }

    pub async fn restore(&self) -> Result<()> {
        let rows = sqlx::query_as::<_, (Uuid, String, String, Vec<u8>)>(
            "SELECT i.source_id,i.tenant_id,i.digest,v.archive FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.enabled",
        )
        .fetch_all(&self.pool)
        .await?;
        for (source, tenant, digest, archive) in rows {
            let bundle = Bundle::decode(&archive)?;
            if bundle.verify()?.manifest().plugin.runtime.process.is_some() {
                self.processes.activate(source, &tenant, &bundle).await?;
                continue;
            }
            let slot = self.slot(source, &tenant).await?;
            if slot
                .snapshot()
                .await?
                .is_some_and(|s| s.bundle.digest() == digest)
            {
                continue;
            }
            // 安装记录是激活提交点，恢复被进程中断的跨库状态变更。
            let grants = bundle.verify()?.manifest().plugin.capabilities.clone();
            let resources = self.resources(source, &tenant, &bundle).await?;
            slot.activate(&self.engine, bundle, grants, resources)
                .await?;
        }
        self.processes.reconcile().await?;
        Ok(())
    }

    async fn validate(&self, source: Uuid, bundle: &Bundle) -> Result<model::Description> {
        let tenant = "component-publication-validation";
        if bundle.verify()?.manifest().plugin.runtime.process.is_some() {
            let (instance, description) = self.processes.prepare(source, tenant, bundle).await?;
            self.processes.stop_id(&instance.start.id()).await?;
            return Ok(description);
        }
        let resources = self.resources(source, tenant, bundle).await?;
        let slot = ComponentSlot::new(
            source,
            tenant.into(),
            semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
        )?;
        let verified = Arc::new(bundle.verify()?);
        let grants = verified.manifest().plugin.capabilities.clone();
        slot.replace(&self.engine, verified, grants, resources)
            .await?;
        let description = slot.snapshot().await.context("候选实例未激活")?.description;
        slot.deactivate().await;
        Ok(description.into())
    }

    pub async fn install(&self, tenant: &str, git: &str, revision: Option<&str>) -> Result<()> {
        let _guard = self.mutations.lock().await;
        let (source, bundle) = self
            .published(git, revision)
            .await?
            .context("插件尚未发布")?;
        self.require_parent(tenant, source).await?;
        if bundle.verify()?.manifest().plugin.runtime.process.is_some() {
            return self.install_process(tenant, source, bundle).await;
        }
        let resources = self.resources(source, tenant, &bundle).await?;
        let grants = bundle.verify()?.manifest().plugin.capabilities.clone();
        let slot = self.slot(source, tenant).await?;
        let previous = slot.stored().await?;
        let permissions: Vec<_> = bundle
            .verify()?
            .manifest()
            .plugin
            .permissions
            .iter()
            .map(|name| services::permission(source, name))
            .collect();
        slot.activate(&self.engine, bundle.clone(), grants, resources)
            .await?;
        let result=async {
            let mut tx=self.pool.begin().await?;
            sqlx::query("INSERT INTO component_installations(tenant_id,source_id,digest,generation) VALUES($1,$2,$3,$4) ON CONFLICT(tenant_id,source_id) DO UPDATE SET digest=EXCLUDED.digest,enabled=true,generation=EXCLUDED.generation")
                .bind(tenant).bind(source).bind(&bundle.digest).bind(Uuid::new_v4()).execute(&mut *tx).await?;
            self.identity.install_permissions(tenant, &permissions).await?;
            tx.commit().await?;
            anyhow::Ok(())
        }.await;
        if let Err(error) = result {
            if let Some(previous) = previous {
                let resources = self.resources(source, tenant, &previous.bundle).await?;
                slot.activate(&self.engine, previous.bundle, previous.grants, resources)
                    .await?;
            } else {
                slot.deactivate().await?;
            }
            return Err(error.into());
        }
        Ok(())
    }

    pub async fn change(&self, tenant: &str, source: Uuid, action: &str) -> Result<()> {
        if action == "enable" {
            let git:String=sqlx::query_scalar("SELECT s.git FROM component_sources s JOIN component_installations i ON i.source_id=s.id WHERE s.id=$1 AND i.tenant_id=$2").bind(source).bind(tenant).fetch_one(&self.pool).await?;
            return self.install(tenant, &git, None).await;
        }
        if action == "rollback" {
            let row=sqlx::query_as::<_,(String,String)>("SELECT s.git,v.digest FROM component_versions v JOIN component_sources s ON s.id=v.source_id JOIN component_installations i ON i.source_id=s.id AND i.tenant_id=$2 JOIN component_versions current ON current.digest=i.digest WHERE s.id=$1 AND v.created_at<current.created_at ORDER BY v.created_at DESC LIMIT 1").bind(source).bind(tenant).fetch_optional(&self.pool).await?.context("没有可回滚的发布版本")?;
            return self.install(tenant, &row.0, Some(&row.1)).await;
        }
        ensure!(action == "disable" || action == "uninstall", "未知管理操作");
        let _guard = self.mutations.lock().await;
        self.require_no_children(tenant, source).await?;
        let installed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations WHERE source_id=$1 AND tenant_id=$2)").bind(source).bind(tenant).fetch_one(&self.pool).await?;
        ensure!(installed, "当前租户未安装插件");
        if let Some(bundle) = self.installed_bundle(tenant, source).await?
            && bundle.verify()?.manifest().plugin.runtime.process.is_some()
        {
            return self.change_process(tenant, source, action, bundle).await;
        }
        let slot = self.slot(source, tenant).await?;
        let previous = slot.stored().await?;
        slot.deactivate().await?;
        let sql = if action == "uninstall" {
            "DELETE FROM component_installations WHERE source_id=$1 AND tenant_id=$2"
        } else {
            "UPDATE component_installations SET enabled=false,generation=gen_random_uuid() WHERE source_id=$1 AND tenant_id=$2"
        };
        let result = sqlx::query(sql)
            .bind(source)
            .bind(tenant)
            .execute(&self.pool)
            .await;
        if let Err(error) = result {
            if let Some(previous) = previous {
                let resources = self.resources(source, tenant, &previous.bundle).await?;
                slot.activate(&self.engine, previous.bundle, previous.grants, resources)
                    .await?;
            }
            return Err(error.into());
        }
        Ok(())
    }
}

fn load_keyring(path: &Path) -> Result<Keyring> {
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
