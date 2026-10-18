use anyhow::{Context, Result, ensure};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_contract::{InvocationScope, RequestContext, process::Configuration};
use az_plugin_runtime::DatabaseEndpoint;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{os::unix::fs::PermissionsExt, sync::Arc};
use tokio::{
    net::{TcpStream, UnixListener, UnixStream},
    task::JoinSet,
};

use super::{
    Processes,
    model::{Gateway, Start},
};

pub(super) fn random() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("生成 process 凭据失败"))?;
    Ok(STANDARD.encode(bytes))
}

impl Processes {
    pub(super) async fn configure(
        &self,
        start: &Start,
        verified: &VerifiedBundle,
        directory: &std::path::Path,
        local: bool,
    ) -> Result<(Configuration, JoinSet<()>)> {
        let components = self.components.upgrade().context("宿主已停止")?;
        let manifest = &verified.manifest().plugin;
        let process = manifest
            .runtime
            .process
            .as_ref()
            .context("不是 process 包")?;
        let approved = std::env::var("AIO_PROCESS_ENDPOINTS").unwrap_or_default();
        ensure!(
            local
                || process
                    .endpoints
                    .iter()
                    .all(|endpoint| approved.split(',').any(|allowed| allowed == endpoint)),
            "process 模型地址未获宿主授权"
        );
        let approved_http = std::env::var("AIO_PROCESS_HTTP_ENDPOINTS").unwrap_or_default();
        let approved_workers = std::env::var("AIO_PROCESS_WORKER_CAPABILITIES").unwrap_or_default();
        ensure!(
            local
                || process.worker_capabilities.iter().all(|capability| {
                    approved_workers
                        .split(',')
                        .any(|approved| approved == capability)
                }),
            "process 设备能力未获宿主授权"
        );
        ensure!(
            local
                || process
                    .http_endpoints
                    .iter()
                    .all(|endpoint| approved_http.split(',').any(|allowed| allowed == endpoint)),
            "process 第三方 HTTP 地址未获宿主授权"
        );
        tokio::fs::create_dir_all(&directory).await?;
        tokio::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700)).await?;
        for name in ["package", "grant", "broker", "database", "runtime"] {
            tokio::fs::create_dir_all(directory.join(name)).await?;
            tokio::fs::set_permissions(
                directory.join(name),
                std::fs::Permissions::from_mode(if name == "runtime" { 0o777 } else { 0o755 }),
            )
            .await?;
        }
        replace_file(
            &directory.join("package/server"),
            verified.component(),
            0o555,
        )
        .await?;
        let scope = InvocationScope {
            source_id: start.source.to_string(),
            revision: String::new(),
            context: RequestContext {
                tenant_id: Some(start.tenant.clone()),
                ..Default::default()
            },
            grants: Default::default(),
        };
        let encryption_key = if manifest.capabilities.cryptography {
            let key = random()?;
            let candidate = components
                .keyring
                .seal(&scope, "process-master", key.as_bytes())?;
            let ciphertext = components
                .catalog
                .process_key(start.source, &start.tenant, candidate)
                .await?;
            Some(String::from_utf8(components.keyring.open(
                &scope,
                "process-master",
                &ciphertext,
            )?)?)
        } else {
            None
        };
        let resources = components
            .resources_verified(start.source, &start.tenant, verified)
            .await?;
        let mut jobs = JoinSet::new();
        let database_url = if manifest.capabilities.database {
            let database = resources.database.context("未授予 process 数据库")?;
            let connection = database.process_connection()?;
            drop(database);
            if local {
                Some(connection.direct_url)
            } else {
                let socket = directory.join("database").join(&connection.socket_name);
                if tokio::fs::try_exists(&socket).await? {
                    tokio::fs::remove_file(&socket).await?;
                }
                let listener = UnixListener::bind(&socket)?;
                tokio::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).await?;
                let upstream = Arc::new(connection.upstream);
                jobs.spawn(async move {
                    let mut clients = JoinSet::new();
                    loop {
                        tokio::select! {
                            result = clients.join_next(), if !clients.is_empty() => {
                                match result {
                                    Some(Ok(Err(error))) => eprintln!("Sidecar 数据库代理失败: {error:#}"),
                                    Some(Err(error)) => eprintln!("Sidecar 数据库代理任务失败: {error}"),
                                    _ => {}
                                }
                            },
                            accepted = listener.accept(), if clients.len() < 6 => {
                                let (local, _) = match accepted {
                                    Ok(client) => client,
                                    Err(error) => {
                                        eprintln!("Sidecar 数据库代理停止接收连接: {error}");
                                        break;
                                    }
                                };
                                clients.spawn(forward_database(local, upstream.clone()));
                            }
                        }
                    }
                });
                Some(connection.isolated_url)
            }
        } else {
            None
        };
        let configuration = Configuration {
            abi_version: 2,
            tenant_id: start.tenant.clone(),
            database_url,
            encryption_key,
            storage: resources.storage.is_some(),
            identity_provider: manifest.capabilities.identity_provider,
            public_routes: manifest.public_routes.clone(),
            public_base: if manifest.public_routes.is_empty() {
                None
            } else {
                Some(format!(
                    "/api/runtime/components/public/{}/{}",
                    start.source,
                    urlencoding::encode(&start.tenant)
                ))
            },
            ingress_token: random()?,
            broker_socket: if local {
                directory
                    .join("broker/gateway.sock")
                    .to_string_lossy()
                    .into()
            } else {
                "/broker/gateway.sock".into()
            },
            endpoints: process.endpoints.clone(),
            http_endpoints: process.http_endpoints.clone(),
            worker_capabilities: process.worker_capabilities.clone(),
            services: process.services.clone(),
        };
        replace_file(
            &directory.join("grant/config.json"),
            &serde_json::to_vec(&configuration)?,
            if local { 0o600 } else { 0o444 },
        )
        .await?;
        let socket = directory.join("broker/gateway.sock");
        if tokio::fs::try_exists(&socket).await? {
            tokio::fs::remove_file(&socket).await?;
        }
        let listener = UnixListener::bind(&socket)?;
        tokio::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).await?;
        let gateway = Arc::new(Gateway {
            components: self.components.clone(),
            start: start.clone(),
            token: configuration.ingress_token.clone(),
            endpoints: configuration.endpoints.clone(),
            http_endpoints: configuration.http_endpoints.clone(),
            worker_capabilities: configuration.worker_capabilities.clone(),
            services: configuration.services.clone(),
            enhancements: manifest.capabilities.enhancements.clone(),
            runtime_capabilities: manifest.capabilities.runtime.clone(),
            management: manifest.capabilities.management,
            identity_provider: manifest.capabilities.identity_provider,
            storage: resources.storage,
            storage_gate: Default::default(),
            stopped: Default::default(),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()?,
            quota: Arc::new(tokio::sync::Semaphore::new(8)),
        });
        // 即使启动任务尚未被轮询就被取消，也撤销已有连接持有的 broker 授权。
        let stopped = gateway.stopped.clone();
        let guard = stopped.clone().drop_guard();
        jobs.spawn(async move {
            let _guard = guard;
            if let Err(error) = axum::serve(listener, super::broker::router(gateway))
                .with_graceful_shutdown(stopped.cancelled_owned())
                .await
            {
                eprintln!("Sidecar broker 服务失败: {error}");
            }
        });
        Ok((configuration, jobs))
    }
}

async fn replace_file(path: &std::path::Path, bytes: &[u8], mode: u32) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode)).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}

// 协议和凭据由数据库后端声明；宿主只负责有界的私有连接转发。
pub(super) async fn forward_database(
    mut local: UnixStream,
    upstream: Arc<DatabaseEndpoint>,
) -> Result<()> {
    let timeout = std::time::Duration::from_secs(3);
    match upstream.as_ref() {
        DatabaseEndpoint::Tcp { host, port } => {
            let mut remote =
                tokio::time::timeout(timeout, TcpStream::connect((host.as_str(), *port)))
                    .await
                    .context("数据库 TCP 连接超时")??;
            tokio::io::copy_bidirectional(&mut local, &mut remote).await?;
        }
        DatabaseEndpoint::Unix(path) => {
            let mut remote = tokio::time::timeout(timeout, UnixStream::connect(path))
                .await
                .context("数据库 Unix 连接超时")??;
            tokio::io::copy_bidirectional(&mut local, &mut remote).await?;
        }
    }
    Ok(())
}
