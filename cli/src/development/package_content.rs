use anyhow::{Context, Result};
use az_plugin_development::{BuildTask, DevConfiguration, PublishedRelease, RunTask};
use az_plugin_package::PluginPackage;
use std::path::Path;

pub(super) enum Package {
    Binary(PluginPackage),
    Component(az_plugin_bundle::Bundle),
}
impl Package {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if let Ok(package) = PluginPackage::decode(bytes) {
            return Ok(Self::Binary(package));
        }
        Ok(Self::Component(az_plugin_bundle::Bundle::decode(bytes)?))
    }
    pub fn metadata(&self) -> Result<PublishedRelease> {
        Ok(match self {
            Self::Binary(package) => PublishedRelease {
                git: package.git.clone(),
                version: package.version.clone(),
                source_sha: package
                    .source_revision
                    .clone()
                    .context("锁定依赖必须关联完整源码 SHA")?,
                digest: package.rev.clone(),
                manifest: package.manifest_toml.clone(),
                abi: 1,
            },
            Self::Component(bundle) => PublishedRelease {
                git: bundle.git.clone(),
                version: bundle.version.clone(),
                source_sha: bundle.commit.clone(),
                digest: bundle.digest.clone(),
                manifest: bundle.manifest.clone(),
                abi: 2,
            },
        })
    }
    pub fn extract(&self, destination: &Path, root: &Path) -> Result<DevConfiguration> {
        let mut prepare = None;
        let (frontend, backend, run) = match self {
            Self::Binary(package) => {
                let verified = package.verify()?;
                let runtime = verified
                    .manifest
                    .plugin
                    .runtime
                    .context("依赖缺少后端运行声明")?;
                let frontend = verified
                    .manifest
                    .plugin
                    .frontend
                    .context("依赖缺少前端运行声明")?;
                write(
                    destination,
                    "aio-plugin.toml",
                    package.manifest_toml.as_bytes(),
                )?;
                write(destination, &runtime.artifact, &verified.artifact)?;
                for (name, bytes) in verified.frontend {
                    write(destination, &format!("{}/{name}", frontend.path), &bytes)?;
                }
                let run = if runtime.kind == az_plugin_manifest::PluginRuntime::Process {
                    let image = runtime
                        .container_image
                        .context("发布进程依赖缺少固定镜像")?;
                    let mut command = vec![
                        "docker".into(),
                        "run".into(),
                        "--rm".into(),
                        "--init".into(),
                        "-p".into(),
                        "127.0.0.1:{port}:8080".into(),
                        "-e".into(),
                        "AIO_PLUGIN_PORT=8080".into(),
                        "-v".into(),
                        format!("{}:/aio:ro", root.display()),
                        "-w".into(),
                        "/aio".into(),
                        image,
                    ];
                    command.extend(runtime.entrypoint.iter().map(|arg| {
                        arg.replace("{artifact}", &format!("/aio/{}", runtime.artifact))
                    }));
                    Some(RunTask {
                        command,
                        debug_arguments: vec![],
                        health: runtime.health_check.unwrap_or_else(|| "/health".into()),
                    })
                } else {
                    None
                };
                (frontend.path, runtime.artifact, run)
            }
            Self::Component(bundle) => {
                let verified = bundle.verify()?;
                let manifest = &verified.manifest().plugin;
                write(destination, "aio-plugin.toml", bundle.manifest.as_bytes())?;
                write(
                    destination,
                    &manifest.runtime.artifact,
                    verified.component(),
                )?;
                for (name, bytes) in verified.frontend_files() {
                    write(
                        destination,
                        &format!("{}/{name}", manifest.frontend.path),
                        bytes,
                    )?;
                }
                if let Some(database) = &manifest.database {
                    for (name, sql) in verified.migrations() {
                        write(
                            destination,
                            &format!("{}/{name}", database.migrations),
                            sql.as_bytes(),
                        )?;
                    }
                }
                let run = if let Some(process) = &manifest.runtime.process {
                    prepare = Some(BuildTask {
                        inputs: vec!["aio-plugin.toml".into()],
                        command: vec![
                            "aio".into(),
                            "plugin".into(),
                            "dev-container-cache".into(),
                            process.image.clone(),
                        ],
                        output: ".aio/dev/container-images".into(),
                    });
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(
                            destination.join(&manifest.runtime.artifact),
                            std::fs::Permissions::from_mode(0o755),
                        )?;
                    }
                    Some(RunTask {
                        command: vec![
                            "aio".into(),
                            "plugin".into(),
                            "dev-container".into(),
                            process.image.clone(),
                            "{artifact}".into(),
                        ],
                        debug_arguments: vec![],
                        health: "/health".into(),
                    })
                } else {
                    None
                };
                (
                    manifest.frontend.path.clone(),
                    manifest.runtime.artifact.clone(),
                    run,
                )
            }
        };
        let task = |output: String| BuildTask {
            inputs: vec![output.clone()],
            command: vec!["true".into()],
            output,
        };
        Ok(DevConfiguration {
            version: 1,
            plugin: None,
            prepare,
            frontend: task(frontend),
            backend: task(backend),
            run,
        })
    }
}
fn write(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    az_plugin_development::validate_relative(name)?;
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().context("产物路径无效")?)?;
    std::fs::write(path, bytes)?;
    Ok(())
}
