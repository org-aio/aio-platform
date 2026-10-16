use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::model::{Start, Stop};
use crate::runtime::server::supervisor::{docker_output, ignore_missing};

fn root() -> Result<PathBuf> {
    Ok(std::env::var_os("AIO_PROCESS_ROOT")
        .map(PathBuf::from)
        .context("监督器未配置 process 私有目录")?
        .canonicalize()?)
}

fn prefix() -> Result<String> {
    let digest = format!(
        "{:x}",
        Sha256::digest(root()?.as_os_str().as_encoded_bytes())
    );
    Ok(format!("aio-v2-{}-", &digest[..8]))
}

pub(in crate::runtime::server) async fn start(request: &Start) -> Result<()> {
    ensure!(
        !request.source.is_nil()
            && !request.tenant.is_empty()
            && request.tenant.len() <= 128
            && request.revision.len() == 64
            && request.revision.bytes().all(|b| b.is_ascii_hexdigit()),
        "process 归属无效"
    );
    let directory = root()?.join(request.id());
    ensure!(
        directory.canonicalize()?.parent() == Some(root()?.as_path()),
        "process 目录无效"
    );
    let bundle = Bundle::decode(&tokio::fs::read(directory.join("bundle.aio-plugin")).await?)?;
    ensure!(bundle.digest == request.revision, "process 包摘要不匹配");
    let verified = bundle.verify()?;
    let manifest = verified.manifest();
    ensure!(
        semver::VersionReq::parse(&manifest.plugin.runtime.host_version)?
            .matches(&semver::Version::parse(env!("CARGO_PKG_VERSION"))?),
        "process 宿主版本不匹配"
    );
    let process = manifest
        .plugin
        .runtime
        .process
        .as_ref()
        .context("不是 process 包")?;
    ensure!(
        std::env::var("AIO_PROCESS_IMAGES")
            .unwrap_or_default()
            .split(',')
            .any(|image| image == process.image),
        "process 运行镜像未获宿主授权"
    );
    let artifact = directory.join("package/server");
    ensure!(
        !tokio::fs::symlink_metadata(&artifact)
            .await?
            .file_type()
            .is_symlink()
            && tokio::fs::read(&artifact).await? == verified.component(),
        "process 可执行文件不属于已校验整包"
    );
    tokio::fs::set_permissions(&artifact, std::fs::Permissions::from_mode(0o555)).await?;
    let name = format!("{}{}", prefix()?, request.id());
    if let Ok(status) = docker_output(["inspect", "--format", "{{.State.Running}}", &name]).await {
        ensure!(status.trim() == "true", "旧 process 实例尚未清理");
        return Ok(());
    }
    docker_output(["image", "inspect", &process.image])
        .await
        .context("process 镜像尚未预置")?;
    // 已确认没有存活容器；回滚、重启或强杀后不能复用旧进程留下的监听文件。
    clear_stale_socket(&directory.join("runtime/service.sock")).await?;
    let args = arguments(&directory, &name, &process.image)?;
    docker_output(args.iter().map(String::as_str))
        .await
        .context("启动 v2 process 失败")?;
    Ok(())
}

pub(in crate::runtime::server) async fn stop(request: &Stop) -> Result<()> {
    ensure!(
        request.id.len() == 24 && request.id.bytes().all(|b| b.is_ascii_hexdigit()),
        "process 实例 ID 无效"
    );
    let name = format!("{}{}", prefix()?, request.id);
    // 先发送 SIGTERM 等待请求排空，超时由 Docker 强制终止；仅忽略已经退出的容器。
    ignore_missing(docker_output(["stop", "--time", "15", &name]).await)?;
    ignore_missing(docker_output(["rm", "--force", &name]).await)?;
    Ok(())
}

async fn clear_stale_socket(socket: &Path) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(socket).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.file_type().is_socket(),
        "process 监听路径不是 Unix socket"
    );
    tokio::fs::remove_file(socket).await?;
    Ok(())
}

pub(in crate::runtime::server) async fn reconcile(requests: &[Start]) -> Result<()> {
    let prefix = prefix()?;
    let active: std::collections::HashSet<_> = requests.iter().map(Start::id).collect();
    let containers = docker_output([
        "ps",
        "--all",
        "--filter",
        "label=io.addzero.aio.v2=true",
        "--format",
        "{{.Names}}",
    ])
    .await?;
    for name in containers.lines() {
        if let Some(id) = name.strip_prefix(&prefix)
            && !active.contains(id)
        {
            stop(&Stop { id: id.into() }).await?;
        }
    }
    Ok(())
}

fn arguments(directory: &std::path::Path, name: &str, image: &str) -> Result<Vec<String>> {
    let path = directory.to_str().context("process 路径无效")?;
    ensure!(!path.contains(','), "process 路径不能包含逗号");
    let mut args: Vec<String> = [
        "run",
        "--detach",
        "--rm",
        "--pull=never",
        "--network=none",
        "--read-only",
        "--init",
        "--user=65532:65532",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges:true",
        "--pids-limit=256",
        "--memory=1073741824",
        "--memory-swap=1073741824",
        "--cpus=2",
        "--ulimit=nofile=1024:1024",
        "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=16777216",
        "--ipc=private",
        "--env=AIO_PLUGIN_CONFIG=/grant/config.json",
        "--env=AIO_PLUGIN_SOCKET=/aio/service.sock",
        "--stop-timeout=15",
        "--log-driver=local",
        "--log-opt=max-size=10m",
        "--log-opt=max-file=2",
        "--label=io.addzero.aio.v2=true",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(format!("--name={name}"));
    for (source, target, readonly) in [
        ("package", "/plugin", true),
        ("grant", "/grant", true),
        ("broker", "/broker", true),
        ("database", "/database", true),
        ("runtime", "/aio", false),
    ] {
        args.push(format!(
            "--mount=type=bind,src={path}/{source},dst={target}{}",
            if readonly { ",readonly" } else { "" }
        ));
    }
    args.push("--entrypoint=/plugin/server".into());
    args.push(image.into());
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_socket_can_rebind_without_removing_files_or_following_links() -> Result<()> {
        let root = tempfile::tempdir()?;
        let socket = root.path().join("service.sock");
        drop(tokio::net::UnixListener::bind(&socket)?);
        clear_stale_socket(&socket).await?;
        drop(tokio::net::UnixListener::bind(&socket)?);
        clear_stale_socket(&socket).await?;
        clear_stale_socket(&socket).await?;

        tokio::fs::write(&socket, b"keep").await?;
        assert!(clear_stale_socket(&socket).await.is_err());
        assert_eq!(tokio::fs::read(&socket).await?, b"keep");
        let link = root.path().join("alias.sock");
        std::os::unix::fs::symlink(&socket, &link)?;
        assert!(clear_stale_socket(&link).await.is_err());
        assert!(
            tokio::fs::symlink_metadata(link)
                .await?
                .file_type()
                .is_symlink()
        );
        Ok(())
    }
    #[test]
    fn container_has_no_network_or_host_credentials_in_arguments() -> Result<()> {
        let args = arguments(
            std::path::Path::new("/private/instance"),
            "aio-v2-test",
            "sha256:abc",
        )?;
        assert!(args.contains(&"--network=none".into()));
        assert!(args.contains(&"--read-only".into()));
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("PASSWORD") || arg.contains("host.docker.internal"))
        );
        assert!(args.contains(&"--entrypoint=/plugin/server".into()));
        Ok(())
    }
}
