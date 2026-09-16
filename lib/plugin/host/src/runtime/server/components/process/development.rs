use super::{
    Processes,
    model::{Instance, Resources, Start},
};
use anyhow::{Context, Result, ensure};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_development::{DevArtifact, DevLaunch};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use uuid::Uuid;

pub(in crate::runtime::server::components) struct SocketAlias {
    path: PathBuf,
    directory: PathBuf,
}
impl Drop for SocketAlias {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl Processes {
    pub(in crate::runtime::server::components) async fn prepare_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<DevLaunch> {
        let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, artifact.source.as_bytes());
        // Unix socket 在 macOS 上只有约 104 字节；短链接指向项目私有状态目录。
        let directory = self
            .root
            .join(format!("development-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&directory)?;
        let alias = SocketAlias {
            directory: directory.clone(),
            path: PathBuf::from(format!("/tmp/aio-{}", Uuid::new_v4().simple())),
        };
        std::os::unix::fs::symlink(&directory, &alias.path)?;
        let start = Start {
            source,
            tenant: "development".into(),
            revision: artifact.backend_digest.clone(),
        };
        let (config, jobs) = self.configure(&start, &bundle, &alias.path, true).await?;
        let socket = alias.path.join("runtime/service.sock");
        let environment = BTreeMap::from([
            (
                "AIO_PLUGIN_CONFIG".into(),
                alias
                    .path
                    .join("grant/config.json")
                    .to_string_lossy()
                    .into(),
            ),
            ("AIO_PLUGIN_SOCKET".into(), socket.to_string_lossy().into()),
        ]);
        let client = reqwest::Client::builder()
            .unix_socket(socket.clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(3))
            .build()?;
        self.pending.lock().await.insert(
            source,
            Arc::new(Instance {
                start,
                bundle,
                token: config.ingress_token,
                client,
                resources: Arc::new(Resources {
                    _jobs: jobs,
                    _socket_alias: Some(alias),
                }),
            }),
        );
        Ok(DevLaunch {
            environment,
            socket: Some(socket),
        })
    }

    pub(in crate::runtime::server::components) async fn activate_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<super::super::model::Description> {
        let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, artifact.source.as_bytes());
        let key = (source, "development".into());
        let existing = self.instances.lock().await.get(&key).cloned();
        let candidate = self
            .pending
            .lock()
            .await
            .get(&source)
            .cloned()
            .or(existing)
            .context("开发服务未准备，请重新构建后端")?;
        ensure!(
            candidate.start.revision == artifact.backend_digest,
            "开发服务构建已过期"
        );
        let response = candidate
            .client
            .get("http://localhost/aio/describe")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;
        let mut description: super::super::model::Description =
            serde_json::from_slice(&super::instance::read_body(response, 128 * 1024).await?)?;
        description.process = true;
        super::instance::validate_description(&bundle, &description)?;
        let description = description.with_settings(&bundle)?;
        self.instances.lock().await.insert(
            key,
            Arc::new(Instance {
                start: candidate.start.clone(),
                bundle,
                token: candidate.token.clone(),
                client: candidate.client.clone(),
                resources: candidate.resources.clone(),
            }),
        );
        self.pending.lock().await.remove(&source);
        Ok(description)
    }
}
