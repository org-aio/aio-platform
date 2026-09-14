use std::{
    collections::HashSet,
    env,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use az_plugin_manifest::{
    PluginRuntime, RuntimeManifest, read_manifest, validate_host_compatibility, validate_repository,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;

const CONTAINER_PORT: u16 = 8080;
const DEFAULT_STOP_TIMEOUT_SECONDS: u64 = 10;
const MAX_DOCKER_OUTPUT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StartProcessRequest {
    pub tenant_id: String,
    pub source_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StopProcessRequest {
    pub instance_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReconcileProcessesRequest {
    pub instances: Vec<StartProcessRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProcessInstance {
    pub instance_id: String,
    pub endpoint: String,
    pub created: bool,
}

#[derive(Clone)]
struct SupervisorState {
    docker: Arc<DockerSupervisor>,
}

pub async fn run() -> Result<()> {
    let socket = env::var_os("AIO_PLUGIN_SUPERVISOR_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/aio-plugin-supervisor/supervisor.sock"));
    let cache_root = env::var_os("AIO_PLUGIN_CACHE")
        .map(PathBuf::from)
        .context("监督器必须显式配置 AIO_PLUGIN_CACHE")?;
    if let Some(parent) = socket.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("创建监督器 socket 目录失败: {}", parent.display()))?;
    }
    if tokio::fs::try_exists(&socket).await? {
        tokio::fs::remove_file(&socket)
            .await
            .with_context(|| format!("清理监督器旧 socket 失败: {}", socket.display()))?;
    }
    let docker = Arc::new(DockerSupervisor::initialize(cache_root).await?);
    let state = SupervisorState { docker };
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/instances/start", post(start))
        .route("/instances/stop", post(stop))
        .route("/instances/reconcile", post(reconcile))
        .route("/bundles/start", post(start_bundle))
        .route("/bundles/stop", post(stop_bundle))
        .route("/bundles/reconcile", post(reconcile_bundles))
        .with_state(state);
    let listener = tokio::net::UnixListener::bind(&socket)
        .with_context(|| format!("绑定监督器 socket 失败: {}", socket.display()))?;
    println!("AIO plugin supervisor listening on {}", socket.display());
    axum::serve(listener, router)
        .await
        .context("AIO 插件监督器异常退出")
}

async fn start(
    State(state): State<SupervisorState>,
    Json(request): Json<StartProcessRequest>,
) -> Result<Json<ProcessInstance>, SupervisorError> {
    state
        .docker
        .start(&request)
        .await
        .map(Json)
        .map_err(Into::into)
}

async fn stop(
    State(state): State<SupervisorState>,
    Json(request): Json<StopProcessRequest>,
) -> Result<StatusCode, SupervisorError> {
    state.docker.stop(&request.instance_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_bundle(
    Json(request): Json<super::components::process::Start>,
) -> Result<StatusCode, SupervisorError> {
    super::components::process::supervision::start(&request).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_bundle(
    Json(request): Json<super::components::process::Stop>,
) -> Result<StatusCode, SupervisorError> {
    super::components::process::supervision::stop(&request).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reconcile_bundles(
    Json(requests): Json<Vec<super::components::process::Start>>,
) -> Result<StatusCode, SupervisorError> {
    super::components::process::supervision::reconcile(&requests).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reconcile(
    State(state): State<SupervisorState>,
    Json(request): Json<ReconcileProcessesRequest>,
) -> Result<StatusCode, SupervisorError> {
    state.docker.reconcile(&request.instances).await?;
    Ok(StatusCode::NO_CONTENT)
}

struct DockerSupervisor {
    cache_root: PathBuf,
    http: reqwest::Client,
}

impl DockerSupervisor {
    async fn initialize(cache_root: PathBuf) -> Result<Self> {
        docker_output(["version", "--format", "{{.Server.Version}}"])
            .await
            .context("连接 Docker 监督后端失败")?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .context("创建插件健康检查客户端失败")?;
        Ok(Self { cache_root, http })
    }

    async fn start(&self, request: &StartProcessRequest) -> Result<ProcessInstance> {
        validate_start_request(request)?;
        let repository = self.cache_root.join(&request.revision);
        let manifest = read_manifest(&repository)?;
        validate_host_compatibility(&manifest, env!("CARGO_PKG_VERSION"))?;
        let report = validate_repository(&repository)?;
        ensure!(
            report.runtime == PluginRuntime::Process,
            "监督器只启动 process 插件"
        );
        ensure!(
            manifest.plugin.capabilities.network.is_empty()
                && manifest.plugin.capabilities.filesystem.is_empty()
                && !manifest.plugin.capabilities.database,
            "当前 process 监督器仅授予零网络、零文件系统和零数据库能力"
        );
        let runtime = manifest
            .plugin
            .runtime
            .as_ref()
            .context("process 插件缺少 runtime 清单")?;
        let image = runtime
            .container_image
            .as_deref()
            .context("process 插件缺少容器镜像")?;
        docker_output(["image", "inspect", image])
            .await
            .with_context(|| format!("未预置 process 插件镜像: {image}"))?;

        let instance_id = instance_id(request);
        if let Some(endpoint) = running_endpoint(&instance_id).await? {
            ensure!(
                self.owns(&instance_id).await?,
                "process 实例属于其他宿主目录"
            );
            if self.healthy(&endpoint, runtime).await {
                return Ok(ProcessInstance {
                    instance_id,
                    endpoint,
                    created: false,
                });
            }
            self.stop(&instance_id).await?;
        }

        let network = network_name(&instance_id);
        ignore_missing(remove_network(&network).await)?;
        docker_output([
            "network",
            "create",
            "--internal",
            "--driver",
            "bridge",
            "--label",
            "io.addzero.aio.managed=true",
            "--label",
            &format!("io.addzero.aio.cache={}", self.cache_root.display()),
            &network,
        ])
        .await
        .context("创建插件隔离网络失败")?;
        let arguments = container_arguments(request, runtime, &repository, &instance_id, &network)?;
        if let Err(error) = docker_output(arguments.iter().map(String::as_str)).await {
            let _ = remove_network(&network).await;
            return Err(error.context("启动插件容器失败"));
        }
        let endpoint = running_endpoint(&instance_id)
            .await?
            .context("插件容器启动后缺少隔离网络地址")?;
        if self.healthy_with_retry(&endpoint, runtime).await {
            return Ok(ProcessInstance {
                instance_id,
                endpoint,
                created: true,
            });
        }
        let logs = docker_output(["logs", "--tail", "80", &container_name(&instance_id)])
            .await
            .unwrap_or_default();
        let _ = self.stop(&instance_id).await;
        bail!("插件容器健康检查超时: {}", truncate(&logs));
    }

    async fn stop(&self, instance_id: &str) -> Result<()> {
        validate_instance_id(instance_id)?;
        match self.owns(instance_id).await {
            Ok(owned) => ensure!(owned, "不能停止其他宿主的 process"),
            Err(error) if missing_docker_object(&error) => {}
            Err(error) => return Err(error),
        }
        ignore_missing(docker_output(["rm", "--force", &container_name(instance_id)]).await)?;
        ignore_missing(remove_network(&network_name(instance_id)).await)?;
        Ok(())
    }

    async fn owns(&self, instance_id: &str) -> Result<bool> {
        let mounts = docker_output([
            "inspect",
            "--format",
            "{{json .Mounts}}",
            &container_name(instance_id),
        ])
        .await?;
        let mounts: Vec<serde_json::Value> = serde_json::from_str(&mounts)?;
        Ok(mounts.iter().any(|mount| {
            mount["Destination"] == "/plugin"
                && mount["Source"].as_str().is_some_and(|source| {
                    Path::new(source).parent() == Some(self.cache_root.as_path())
                })
        }))
    }

    async fn reconcile(&self, instances: &[StartProcessRequest]) -> Result<()> {
        let mut active = HashSet::with_capacity(instances.len());
        for instance in instances {
            validate_start_request(instance)?;
            active.insert(instance_id(instance));
        }
        let containers = docker_output([
            "ps",
            "--all",
            "--filter",
            "label=io.addzero.aio.managed=true",
            "--format",
            "{{.Names}}",
        ])
        .await?;
        for instance_id in orphan_instance_ids(&containers, "aio-plugin-", &active) {
            if self.owns(&instance_id).await? {
                self.stop(&instance_id).await?;
            }
        }
        let networks = docker_output([
            "network",
            "ls",
            "--filter",
            "label=io.addzero.aio.managed=true",
            "--filter",
            &format!("label=io.addzero.aio.cache={}", self.cache_root.display()),
            "--format",
            "{{.Name}}",
        ])
        .await?;
        for instance_id in orphan_instance_ids(&networks, "aio-plugin-net-", &active) {
            ignore_missing(remove_network(&network_name(&instance_id)).await)?;
        }
        Ok(())
    }

    async fn healthy_with_retry(&self, endpoint: &str, runtime: &RuntimeManifest) -> bool {
        for _ in 0..30 {
            if self.healthy(endpoint, runtime).await {
                return true;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        false
    }

    async fn healthy(&self, endpoint: &str, runtime: &RuntimeManifest) -> bool {
        let Some(path) = runtime.health_check.as_deref() else {
            return false;
        };
        self.http
            .get(format!("{endpoint}{path}"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }
}

fn validate_start_request(request: &StartProcessRequest) -> Result<()> {
    ensure!(
        !request.tenant_id.is_empty()
            && request.tenant_id.len() <= 128
            && request
                .tenant_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "租户 id 格式无效"
    );
    uuid::Uuid::parse_str(&request.source_id).context("插件来源 id 必须是 UUID")?;
    ensure!(
        super::repository::is_artifact_revision(&request.revision),
        "插件 revision 必须是完整 Git SHA 或插件包 SHA-256"
    );
    Ok(())
}

fn validate_instance_id(instance_id: &str) -> Result<()> {
    ensure!(
        instance_id.len() == 24 && instance_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "进程插件实例 id 格式无效"
    );
    Ok(())
}

fn orphan_instance_ids(names: &str, prefix: &str, active: &HashSet<String>) -> Vec<String> {
    names
        .lines()
        .filter_map(|name| name.trim().strip_prefix(prefix))
        .filter(|instance_id| {
            validate_instance_id(instance_id).is_ok() && !active.contains(*instance_id)
        })
        .map(str::to_owned)
        .collect()
}

fn instance_id(request: &StartProcessRequest) -> String {
    let mut digest = Sha256::new();
    digest.update(request.tenant_id.as_bytes());
    digest.update([0]);
    digest.update(request.source_id.as_bytes());
    digest.update([0]);
    digest.update(request.revision.as_bytes());
    format!("{:x}", digest.finalize())[..24].to_owned()
}

fn container_name(instance_id: &str) -> String {
    format!("aio-plugin-{instance_id}")
}

fn network_name(instance_id: &str) -> String {
    format!("aio-plugin-net-{instance_id}")
}

fn container_arguments(
    request: &StartProcessRequest,
    runtime: &RuntimeManifest,
    repository: &Path,
    instance_id: &str,
    network: &str,
) -> Result<Vec<String>> {
    let repository = repository
        .canonicalize()
        .with_context(|| format!("解析插件仓库路径失败: {}", repository.display()))?;
    let repository = repository.to_str().context("插件仓库路径不是 Unicode")?;
    ensure!(!repository.contains(','), "插件仓库路径不能包含逗号");
    let image = runtime
        .container_image
        .as_deref()
        .context("process 插件缺少容器镜像")?;
    let stop_timeout = runtime
        .shutdown_timeout_seconds
        .unwrap_or(DEFAULT_STOP_TIMEOUT_SECONDS)
        .to_string();
    let name = container_name(instance_id);
    let mount = format!("type=bind,src={repository},dst=/plugin,readonly");
    let labels = [
        "io.addzero.aio.managed=true".to_owned(),
        format!("io.addzero.aio.tenant={}", request.tenant_id),
        format!("io.addzero.aio.source={}", request.source_id),
        format!("io.addzero.aio.revision={}", request.revision),
    ];
    let mut arguments = vec![
        "run".to_owned(),
        "--detach".to_owned(),
        "--rm".to_owned(),
        "--pull=never".to_owned(),
        format!("--name={name}"),
        format!("--hostname={name}"),
        format!("--network={network}"),
        "--read-only".to_owned(),
        "--init".to_owned(),
        "--user=65532:65532".to_owned(),
        "--cap-drop=ALL".to_owned(),
        "--security-opt=no-new-privileges:true".to_owned(),
        "--pids-limit=128".to_owned(),
        "--memory=268435456".to_owned(),
        "--memory-swap=268435456".to_owned(),
        "--cpus=0.50".to_owned(),
        "--ulimit=nofile=1024:1024".to_owned(),
        "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=16777216".to_owned(),
        "--ipc=private".to_owned(),
        format!("--mount={mount}"),
        "--workdir=/plugin".to_owned(),
        format!("--env=AIO_PLUGIN_PORT={CONTAINER_PORT}"),
        "--env=AIO_PLUGIN_DEFINITION_PATH=/aio/definition".to_owned(),
        format!("--stop-timeout={stop_timeout}"),
        "--log-driver=local".to_owned(),
        "--log-opt=max-size=10m".to_owned(),
        "--log-opt=max-file=2".to_owned(),
    ];
    for label in labels {
        arguments.push(format!("--label={label}"));
    }
    arguments.push(image.to_owned());
    arguments.extend(runtime.entrypoint.iter().map(|argument| {
        if argument == "{artifact}" {
            format!("/plugin/{}", runtime.artifact)
        } else {
            argument.clone()
        }
    }));
    Ok(arguments)
}

async fn running_endpoint(instance_id: &str) -> Result<Option<String>> {
    let name = container_name(instance_id);
    let output = match docker_output([
        "inspect",
        "--format",
        "{{.State.Running}} {{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
        &name,
    ])
    .await
    {
        Ok(output) => output,
        Err(error) if missing_docker_object(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut fields = output.split_whitespace();
    if fields.next() != Some("true") {
        return Ok(None);
    }
    let ip = fields.next().context("插件容器缺少 IP")?;
    let ip = ip.parse::<IpAddr>().context("插件容器 IP 无效")?;
    Ok(Some(format!("http://{ip}:{CONTAINER_PORT}")))
}

async fn remove_network(network: &str) -> Result<String> {
    docker_output(["network", "rm", network]).await
}

pub(super) async fn docker_output<'a>(
    arguments: impl IntoIterator<Item = &'a str>,
) -> Result<String> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let output = tokio::time::timeout(
        Duration::from_secs(60),
        Command::new("docker")
            .args(&arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .with_context(|| format!("Docker 命令超时: {}", arguments.join(" ")))??;
    let stdout = truncate(&String::from_utf8_lossy(&output.stdout));
    if !output.status.success() {
        let stderr = truncate(&String::from_utf8_lossy(&output.stderr));
        bail!("Docker 命令失败: {stderr}");
    }
    Ok(stdout)
}

fn ignore_missing(result: Result<String>) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(error) if missing_docker_object(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn missing_docker_object(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("no such container")
        || message.contains("no such network")
        || message.contains("no such object")
        || message.contains("not found")
}

fn truncate(value: &str) -> String {
    value.chars().take(MAX_DOCKER_OUTPUT_BYTES).collect()
}

struct SupervisorError(anyhow::Error);

impl<E> From<E> for SupervisorError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self(value.into())
    }
}

impl IntoResponse for SupervisorError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("{:#}", self.0) })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn builds_locked_down_container_arguments() -> Result<()> {
        let repository = tempdir()?;
        let runtime = RuntimeManifest {
            kind: PluginRuntime::Process,
            artifact: "dist/plugin.js".to_owned(),
            host_version: None,
            container_image: Some(
                "node:22@sha256:6c74791e557ce11fc957704f6d4fe134a7bc8d6f5ca4403205b2966bd488f6b3"
                    .to_owned(),
            ),
            entrypoint: vec!["node".to_owned(), "{artifact}".to_owned()],
            health_check: Some("/health".to_owned()),
            shutdown_timeout_seconds: Some(8),
        };
        let request = StartProcessRequest {
            tenant_id: "default".to_owned(),
            source_id: "78e88a28-1c20-4f96-b486-a242d4cb28c0".to_owned(),
            revision: "8".repeat(40),
        };
        let arguments = container_arguments(
            &request,
            &runtime,
            repository.path(),
            "0123456789abcdef01234567",
            "aio-plugin-net-0123456789abcdef01234567",
        )?;

        for required in [
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges:true",
            "--user=65532:65532",
            "--memory=268435456",
            "--network=aio-plugin-net-0123456789abcdef01234567",
            "/plugin/dist/plugin.js",
        ] {
            assert!(arguments.iter().any(|argument| argument == required));
        }
        assert!(!arguments.iter().any(|argument| argument == "--privileged"));
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with("--publish"))
        );
        Ok(())
    }

    #[test]
    fn derives_stable_tenant_scoped_instance_id() {
        let first = StartProcessRequest {
            tenant_id: "default".to_owned(),
            source_id: "78e88a28-1c20-4f96-b486-a242d4cb28c0".to_owned(),
            revision: "8".repeat(40),
        };
        let mut second = first.clone();
        second.tenant_id = "another".to_owned();
        assert_eq!(instance_id(&first), instance_id(&first));
        assert_ne!(instance_id(&first), instance_id(&second));
    }

    #[test]
    fn accepts_full_package_revisions_without_allowing_cache_path_traversal() {
        let mut request = StartProcessRequest {
            tenant_id: "default".to_owned(),
            source_id: "78e88a28-1c20-4f96-b486-a242d4cb28c0".to_owned(),
            revision: "abcdef01".repeat(8),
        };
        assert!(validate_start_request(&request).is_ok());
        let package_instance = instance_id(&request);
        request.revision = "abcdef01".repeat(5);
        assert!(validate_start_request(&request).is_ok());
        assert_ne!(instance_id(&request), package_instance);

        for revision in [
            String::new(),
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
            format!("../{}", "a".repeat(64)),
            format!("/{}", "a".repeat(64)),
            format!("{}\\..\\outside", "a".repeat(64)),
            format!("{}%2f..%2foutside", "a".repeat(64)),
        ] {
            request.revision = revision;
            assert!(
                validate_start_request(&request).is_err(),
                "invalid cache revision was accepted: {:?}",
                request.revision
            );
        }
    }

    #[test]
    fn accepts_docker_missing_object_variants() {
        for message in [
            "Error: No such container: example",
            "Error: No such network: example",
            "Error: No such object: example",
            "Error response from daemon: network example not found",
        ] {
            assert!(missing_docker_object(&anyhow::anyhow!(message)));
        }
    }

    #[test]
    fn finds_only_managed_orphan_instance_names() {
        let active = ["0123456789abcdef01234567".to_owned()]
            .into_iter()
            .collect();
        let names = "aio-plugin-0123456789abcdef01234567\naio-plugin-89abcdef0123456789abcdef\naio-plugin-invalid\nother-89abcdef0123456789abcdef";

        assert_eq!(
            orphan_instance_ids(names, "aio-plugin-", &active),
            vec!["89abcdef0123456789abcdef"]
        );
    }
}
