use super::{
    dependencies::Workspace,
    options::Options,
    process::{self, OwnedProcess},
};
use anyhow::{Context, Result, ensure};
use az_plugin_development::{DevArtifact, fingerprint, source_identity};
use std::{collections::BTreeMap, path::Path, time::Duration};

pub(super) struct RunningPlugin {
    pub workspace: Workspace,
    pub frontend_input: Option<String>,
    pub backend_input: Option<String>,
    pub process: Option<OwnedProcess>,
    pub endpoint: Option<String>,
    pub crashed: bool,
    pub attempted: Option<(String, String)>,
}

impl RunningPlugin {
    pub fn inputs(&self) -> Result<(String, String)> {
        let root = &self.workspace.root;
        let common = ["aio-dev.toml".into(), "aio-plugin.toml".into()];
        let config = if self.workspace.published {
            self.workspace.config.clone()
        } else {
            az_plugin_development::read(root)?
        };
        let prepare = config
            .prepare
            .as_ref()
            .map(|task| task.inputs.clone())
            .unwrap_or_default();
        Ok((
            fingerprint(
                root,
                &[common.as_slice(), &prepare, &config.frontend.inputs].concat(),
            )?,
            fingerprint(
                root,
                &[common.as_slice(), &prepare, &config.backend.inputs].concat(),
            )?,
        ))
    }

    pub async fn update(
        &mut self,
        options: &Options,
        state_root: &Path,
        host: &str,
        token: &str,
        generation: u64,
    ) -> Result<bool> {
        let (front, back) = self.inputs()?;
        self.attempted = Some((front.clone(), back.clone()));
        if self.frontend_input.as_ref() == Some(&front)
            && self.backend_input.as_ref() == Some(&back)
        {
            return Ok(false);
        }
        if !self.workspace.published {
            self.workspace.config = az_plugin_development::read(&self.workspace.root)?;
        }
        let config = &self.workspace.config;
        let root = &self.workspace.root;
        let logs = state_root.join("logs");
        let name = root.file_name().context("插件目录无效")?.to_string_lossy();
        let front_changed = self.frontend_input.as_ref() != Some(&front);
        let back_changed = self.crashed || self.backend_input.as_ref() != Some(&back);
        let cache_dir = root.join(".aio/dev");
        std::fs::create_dir_all(&cache_dir)?;
        let cache_path = cache_dir.join("build-state.json");
        let mut cache: BTreeMap<String, String> = std::fs::read(&cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if let Some(task) = &config.prepare {
            let input = fingerprint(root, &task.inputs)?;
            if cache.get("prepare") != Some(&input) || !root.join(&task.output).exists() {
                process::build(
                    &task.command,
                    root,
                    &logs.join(format!("{name}-prepare.log")),
                )
                .await?;
                cache.insert("prepare".into(), input);
                std::fs::write(&cache_path, serde_json::to_vec(&cache)?)?;
            }
        }
        for (key, changed, input, task) in [
            ("frontend", front_changed, &front, &config.frontend),
            ("backend", back_changed, &back, &config.backend),
        ] {
            if changed && (cache.get(key) != Some(input) || !root.join(&task.output).exists()) {
                process::build(&task.command, root, &logs.join(format!("{name}-{key}.log")))
                    .await?;
                cache.insert(key.into(), input.clone());
                std::fs::write(&cache_path, serde_json::to_vec(&cache)?)?;
            }
        }
        // 构建期间的连续保存优先合并，避免启动马上过期的服务。
        ensure!(
            self.inputs()? == (front.clone(), back.clone()),
            "源码继续变化，合并到下一轮构建"
        );
        let digest = az_plugin_development::artifact_digest(
            &root.join("aio-plugin.toml"),
            &root.join(&config.frontend.output),
            &root.join(&config.backend.output),
        )?;
        let backend_digest =
            az_plugin_development::backend_digest(&root.join(&config.backend.output))?;
        let mut descriptor = DevArtifact {
            workspace: root.clone(),
            source: source_identity(root)?,
            content_digest: digest,
            frontend: root.join(&config.frontend.output),
            backend: root.join(&config.backend.output),
            backend_digest,
            endpoint: self.endpoint.clone(),
            generation,
        };
        let mut candidate = None;
        let mut endpoint = self.endpoint.clone();
        if back_changed && let Some(run) = &config.run {
            let port = process::port()?;
            let debug_port = process::port()?;
            let mut debug_args = if options.debug {
                run.debug_arguments
                    .iter()
                    .map(|s| s.replace("{debug_port}", &debug_port.to_string()))
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            for argument in &options.jvm_args {
                debug_args.extend(["--jvm-args".into(), argument.clone()]);
            }
            let response = reqwest::Client::new()
                .post(format!("{host}/api/development/prepare"))
                .bearer_auth(token)
                .json(&descriptor)
                .send()
                .await?;
            let status = response.status();
            let body = response.text().await?;
            ensure!(status.is_success(), "开发服务准备失败: {body}");
            let launch: az_plugin_development::DevLaunch = serde_json::from_str(&body)?;
            let artifact = root.join(&config.backend.output);
            let mut command = vec![];
            for arg in &run.command {
                if arg == "{debug_args}" {
                    command.extend(debug_args.clone());
                } else {
                    command.push(
                        arg.replace("{artifact}", &artifact.to_string_lossy())
                            .replace("{port}", &port.to_string())
                            .replace(
                                "{runtime_dir}",
                                &launch
                                    .socket
                                    .as_deref()
                                    .and_then(|path| path.parent())
                                    .and_then(|path| path.parent())
                                    .map(|path| path.to_string_lossy())
                                    .unwrap_or_default(),
                            ),
                    );
                }
            }
            let mut env = launch.environment;
            env.insert("AIO_PLUGIN_PORT".into(), port.to_string());
            env.insert("AIO_PLUGIN_HOST".into(), "127.0.0.1".into());
            let mut process = process::spawn(
                &command,
                root,
                &env,
                &logs.join(format!("{name}-service.log")),
            )?;
            let address = format!("http://127.0.0.1:{port}");
            ready(
                &format!("{address}{}", run.health),
                &mut process,
                launch.socket.as_deref(),
            )
            .await?;
            if options.debug && !debug_args.is_empty() {
                super::debugger::backend(root, debug_port, &debug_args)?;
                println!("{name} 调试端口: 127.0.0.1:{debug_port}");
            }
            candidate = Some(process);
            endpoint = Some(address);
        }
        // 保存期间再次变化则丢弃本轮结果，不能覆盖更晚的源码目标。
        ensure!(
            self.inputs()? == (front.clone(), back.clone()),
            "源码继续变化，合并到下一轮构建"
        );
        descriptor.endpoint = endpoint.clone();
        let response = reqwest::Client::new()
            .post(format!("{host}/api/development/activate"))
            .bearer_auth(token)
            .json(&descriptor)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        ensure!(status.is_success(), "开发激活失败: {body}");
        if candidate.is_some() {
            self.process = candidate;
            self.crashed = false;
        }
        self.endpoint = endpoint;
        self.frontend_input = Some(front);
        self.backend_input = Some(back);
        println!(
            "已加载 {name}，开发版本 {}",
            &descriptor.content_digest[..12]
        );
        Ok(true)
    }
}

pub(super) async fn ready(
    url: &str,
    process: &mut OwnedProcess,
    socket: Option<&Path>,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2));
    #[cfg(unix)]
    let client = if let Some(socket) = socket {
        client.unix_socket(socket)
    } else {
        client
    };
    let client = client.build()?;
    for _ in 0..120 {
        if let Some(exit) = process.child.try_wait()? {
            anyhow::bail!("插件服务启动时退出 ({exit})，请查看 service.log");
        }
        if client
            .get(url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    anyhow::bail!("服务未在 60 秒内通过健康检查: {url}")
}

pub(super) fn entry(workspace: Workspace) -> RunningPlugin {
    RunningPlugin {
        workspace,
        frontend_input: None,
        backend_input: None,
        process: None,
        endpoint: None,
        crashed: false,
        attempted: None,
    }
}
