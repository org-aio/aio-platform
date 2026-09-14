mod adapter;
mod container;
mod database;
pub use container::{cache as cache_container, run as run_container};
mod debugger;
mod dependencies;
mod distribution;
mod options;
mod package_content;
mod packages;
mod process;
mod rust;
mod tasks;
pub use adapter::build as build_adapter;

use anyhow::{Context, Result};
use az_plugin_development::{DevHostSession, DevStatus};
use std::{collections::BTreeMap, time::Duration};

pub fn run(args: &[String]) -> Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!(
            "aio plugin dev [.] [--debug] [--with <依赖路径>] [--no-watch] [--no-open] [--offline] [--port <端口>] [--database-url <独立开发数据库>] [--jvm-args <参数>]"
        );
        return Ok(());
    }
    let options = options::parse(args)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            tokio::select! {
                result = develop(options) => result,
                _ = termination() => { println!("正在停止本次开发进程，保留开发数据"); Ok(()) }
            }
        })
}

async fn termination() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn develop(options: options::Options) -> Result<()> {
    let executable = distribution::host()?;
    let root = options.root.join(".aio/dev");
    std::fs::create_dir_all(root.join("logs"))?;
    let ownership = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(root.join("run.lock"))?;
    ownership
        .try_lock()
        .context("当前插件已经有一个开发沙箱在运行")?;
    let workspace = options.root.clone();
    let overrides = options.overrides.clone();
    let offline = options.offline;
    let (workspaces, lock) =
        tokio::task::spawn_blocking(move || dependencies::resolve(&workspace, &overrides, offline))
            .await??;
    std::fs::write(
        options.root.join("aio-dev.lock"),
        serde_json::to_vec_pretty(&lock)?,
    )?;
    let database = database::prepare(&root, options.database.as_deref()).await?;
    let session = DevHostSession {
        database_url: database.url.clone(),
        root: root.clone(),
        port: options.port,
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
        workspaces: workspaces
            .iter()
            .zip(&lock.plugins)
            .map(|(workspace, locked)| az_plugin_development::DevWorkspace {
                path: workspace.root.clone(),
                source: workspace.source.clone(),
                version: locked.version.clone().unwrap_or_default(),
            })
            .collect(),
    };
    let session_path = root.join("session.json");
    process::private_file(&session_path, &serde_json::to_vec(&session)?)?;
    let host_info = root.join("host.json");
    if host_info.exists() {
        std::fs::remove_file(&host_info)?;
    }
    let mut host_process = process::spawn(
        &[
            executable.to_string_lossy().into(),
            "--session".into(),
            session_path.to_string_lossy().into(),
        ],
        &options.root,
        &BTreeMap::new(),
        &root.join("logs/host.log"),
    )?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let url = loop {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "开发宿主 60 秒内未就绪；查看 {}",
            root.join("logs/host.log").display()
        );
        if let Some(exit) = host_process.child.try_wait()? {
            anyhow::bail!(
                "开发宿主退出 ({exit})；查看 {}",
                root.join("logs/host.log").display()
            );
        }
        if let Ok(bytes) = std::fs::read(&host_info)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(url) = value["url"].as_str()
        {
            break url.to_owned();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    if options.database.is_some() {
        process::private_file(
            &root.join("database-connection.json"),
            &serde_json::to_vec(&database.url)?,
        )?;
    }
    println!("开发壳: {url}\n日志: {}", root.join("logs").display());
    if options.debug {
        debugger::browser(&options, &url)?;
    }
    if options.open {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(opener).arg(&url).spawn();
    }
    let dependencies = lock
        .plugins
        .iter()
        .map(|plugin| (plugin.source.clone(), plugin.dependencies.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut plugins = workspaces.into_iter().map(tasks::entry).collect::<Vec<_>>();
    let mut generation = 0_u64;
    let mut revisions = BTreeMap::new();
    let mut failures = BTreeMap::new();
    let mut first_build = true;
    loop {
        if let Some(exit) = host_process.child.try_wait()? {
            anyhow::bail!(
                "开发宿主退出 ({exit})，查看 {}",
                root.join("logs/host.log").display()
            );
        }
        generation += 1;
        for plugin in &mut plugins {
            if !plugin.crashed
                && let Some(process) = plugin.process.as_mut()
                && let Some(exit) = process.child.try_wait()?
            {
                plugin.crashed = true;
                let message = format!(
                    "{} 的后端已退出 ({exit})，修改源码后重试；查看 service.log",
                    plugin.workspace.root.display()
                );
                eprintln!("{message}");
                failures.insert(plugin.workspace.source.clone(), message);
                report_outcome(&url, &session.token, generation, &revisions, &failures).await?;
            }
            if let Some(missing) = dependencies[&plugin.workspace.source]
                .iter()
                .find(|source| !revisions.contains_key(*source))
            {
                let message = format!(
                    "依赖 {missing} 尚未成功启动，等待依赖构建后加载 {}",
                    plugin.workspace.source
                );
                if failures.get(&plugin.workspace.source) != Some(&message) {
                    failures.insert(plugin.workspace.source.clone(), message);
                    report_outcome(&url, &session.token, generation, &revisions, &failures).await?;
                }
                continue;
            }
            if !options.watch && !first_build {
                continue;
            }
            let inputs = match plugin.inputs() {
                Ok(inputs) => inputs,
                Err(error) => {
                    let message = format!("{error:#}");
                    if failures.get(&plugin.workspace.source) != Some(&message) {
                        eprintln!("{message}\n继续使用上一成功版本");
                        failures.insert(plugin.workspace.source.clone(), message);
                        report_outcome(&url, &session.token, generation, &revisions, &failures)
                            .await?;
                    }
                    continue;
                }
            };
            if plugin.frontend_input.as_ref() == Some(&inputs.0)
                && plugin.backend_input.as_ref() == Some(&inputs.1)
            {
                plugin.attempted = Some(inputs);
                if !plugin.crashed && failures.remove(&plugin.workspace.source).is_some() {
                    report_outcome(&url, &session.token, generation, &revisions, &failures).await?;
                }
                continue;
            }
            if plugin.attempted.as_ref() == Some(&inputs) {
                continue;
            }
            // 等待一次安静窗口，将编辑器连续写入合成一个构建目标。
            tokio::time::sleep(Duration::from_millis(250)).await;
            if plugin.inputs().ok().as_ref() != Some(&inputs) {
                continue;
            }
            let name = plugin.workspace.root.display().to_string();
            report(
                &url,
                &session.token,
                generation,
                "building",
                &name,
                &revisions,
            )
            .await?;
            match plugin
                .update(&options, &root, &url, &session.token, generation)
                .await
            {
                Ok(true) => {
                    revisions.insert(
                        plugin.workspace.source.clone(),
                        format!("{}:{}", inputs.0, inputs.1),
                    );
                    failures.remove(&plugin.workspace.source);
                    report_outcome(&url, &session.token, generation, &revisions, &failures).await?;
                }
                Ok(false) => {}
                Err(error) => {
                    eprintln!("{error:#}\n继续使用上一成功版本");
                    failures.insert(plugin.workspace.source.clone(), format!("{error:#}"));
                    report_outcome(&url, &session.token, generation, &revisions, &failures).await?;
                }
            }
        }
        first_build = false;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

async fn report(
    url: &str,
    token: &str,
    generation: u64,
    phase: &str,
    message: &str,
    revisions: &BTreeMap<String, String>,
) -> Result<()> {
    reqwest::Client::new()
        .post(format!("{url}/api/development/status"))
        .bearer_auth(token)
        .json(&DevStatus {
            generation,
            phase: phase.into(),
            message: message.into(),
            revisions: revisions.clone(),
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn report_outcome(
    url: &str,
    token: &str,
    generation: u64,
    revisions: &BTreeMap<String, String>,
    failures: &BTreeMap<String, String>,
) -> Result<()> {
    let phase = if failures.is_empty() {
        "ready"
    } else {
        "failed"
    };
    let message = if failures.is_empty() {
        "构建完成".into()
    } else {
        failures.values().cloned().collect::<Vec<_>>().join("\n")
    };
    report(url, token, generation, phase, &message, revisions).await
}
