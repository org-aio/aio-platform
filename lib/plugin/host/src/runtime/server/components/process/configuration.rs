use anyhow::{Context, Result, ensure};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_contract::{InvocationScope, RequestContext, process::Configuration};
use base64::{Engine, engine::general_purpose::STANDARD};
use sqlx::ConnectOptions;
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
        ensure!(
            !manifest.capabilities.storage
                && !manifest.capabilities.management
                && !manifest.capabilities.identity_provider,
            "process 未开放存储、管理及身份提供能力"
        );
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
            sqlx::query("INSERT INTO component_process_keys(source_id,tenant_id,ciphertext) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                .bind(start.source).bind(&start.tenant).bind(components.keyring.seal(&scope, "process-master", key.as_bytes())?).execute(&components.pool).await?;
            let ciphertext: Vec<u8> = sqlx::query_scalar(
                "SELECT ciphertext FROM component_process_keys WHERE source_id=$1 AND tenant_id=$2",
            )
            .bind(start.source)
            .bind(&start.tenant)
            .fetch_one(&components.pool)
            .await?;
            Some(String::from_utf8(components.keyring.open(
                &scope,
                "process-master",
                &ciphertext,
            )?)?)
        } else {
            None
        };
        let mut jobs = JoinSet::new();
        let database_url = if manifest.capabilities.database {
            let migrations = verified
                .migrations()
                .map(|(name, sql)| (name.to_owned(), sql.to_owned()))
                .collect::<Vec<_>>();
            let database = components
                .provisioner
                .install(
                    &start.source.to_string(),
                    &start.tenant,
                    &migrations,
                    &components.keyring,
                )
                .await?;
            drop(database);
            let connection = components
                .provisioner
                .process_connection(
                    &start.source.to_string(),
                    &start.tenant,
                    &components.keyring,
                )
                .await?;
            if local {
                Some(connection.to_url_lossy().to_string())
            } else {
                let socket = directory.join("database/.s.PGSQL.5432");
                if tokio::fs::try_exists(&socket).await? {
                    tokio::fs::remove_file(&socket).await?;
                }
                let listener = UnixListener::bind(&socket)?;
                tokio::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666)).await?;
                let host = connection.get_host().to_owned();
                let port = connection.get_port();
                let upstream_socket = connection
                    .get_socket()
                    .map(|path| path.join(format!(".s.PGSQL.{port}")));
                eprintln!(
                    "process 数据库代理上游 host={host} port={port} socket={upstream_socket:?}"
                );
                jobs.spawn(async move {
                let mut clients = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = clients.join_next(), if !clients.is_empty() => {},
                        accepted = listener.accept(), if clients.len() < 6 => {
                            let Ok((mut local, _)) = accepted else { break };
                            let host = host.clone();
                            let socket = upstream_socket.clone();
                            clients.spawn(async move {
                                if let Some(socket) = socket {
                                    match UnixStream::connect(&socket).await {
                                        Ok(mut remote) => {
                                            let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
                                            return;
                                        }
                                        Err(error) => {
                                            eprintln!("process 数据库 Unix socket 转发失败 {}: {error}", socket.display());
                                        }
                                    }
                                }
                                match TcpStream::connect((host.as_str(), port)).await {
                                    Ok(mut remote) => {
                                        let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
                                    }
                                    Err(error) => {
                                        eprintln!("process 数据库 TCP 转发失败 {host}:{port}: {error}");
                                    }
                                }
                            });
                        }
                    }
                }
            });
                let mut url = connection.to_url_lossy();
                url.set_host(Some("localhost"))?;
                url.set_port(Some(5432))
                    .map_err(|_| anyhow::anyhow!("数据库端口无效"))?;
                url.query_pairs_mut().append_pair("host", "/database");
                Some(url.to_string())
            }
        } else {
            None
        };
        let configuration = Configuration {
            abi_version: 2,
            tenant_id: start.tenant.clone(),
            database_url,
            encryption_key,
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
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()?,
            quota: Arc::new(tokio::sync::Semaphore::new(8)),
        });
        jobs.spawn(async move {
            let _ = axum::serve(listener, super::broker::router(gateway)).await;
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
