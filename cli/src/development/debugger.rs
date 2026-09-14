use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

pub(super) fn browser(options: &super::options::Options, url: &str) -> Result<()> {
    let root = &options.root;
    let executable = std::env::current_exe()?;
    let mut command = vec![
        shell(&executable.to_string_lossy()),
        "plugin dev . --debug --no-open".into(),
    ];
    for path in &options.overrides {
        command.extend(["--with".into(), shell(&path.to_string_lossy())]);
    }
    if options.offline {
        command.push("--offline".into());
    }
    if options.port != 0 {
        command.extend(["--port".into(), options.port.to_string()]);
    }
    for argument in &options.jvm_args {
        command.extend(["--jvm-args".into(), shell(argument)]);
    }
    write(
        root,
        "Sandbox",
        &format!(
            r#"<configuration default="false" name="AIO Sandbox" type="ShConfigurationType"><option name="SCRIPT_TEXT" value="{}"/><option name="INDEPENDENT_SCRIPT_PATH" value="true"/><option name="SCRIPT_WORKING_DIRECTORY" value="$PROJECT_DIR$"/><option name="EXECUTE_IN_TERMINAL" value="true"/><option name="EXECUTE_SCRIPT_FILE" value="false"/></configuration>"#,
            xml(&command.join(" "))
        ),
    )?;
    write(
        root,
        "Frontend",
        &format!(
            r#"<configuration default="false" name="AIO Frontend" type="JavascriptDebugType" uri="{}"><method v="2"/></configuration>"#,
            xml(url)
        ),
    )?;
    Ok(())
}

pub(super) fn backend(root: &Path, port: u16, arguments: &[String]) -> Result<()> {
    let jvm = arguments.iter().any(|argument| argument.contains("jdwp"));
    if !jvm
        && !arguments
            .iter()
            .any(|argument| argument.contains("--inspect"))
    {
        return Ok(());
    }
    if jvm {
        write(
            root,
            "JVM",
            &format!(
                r#"<configuration default="false" name="AIO JVM Attach" type="Remote" factoryName="Remote JVM Debug"><option name="USE_SOCKET_TRANSPORT" value="true"/><option name="SERVER_MODE" value="false"/><option name="SHMEM_ADDRESS" value="javadebug"/><option name="HOST" value="127.0.0.1"/><option name="PORT" value="{port}"/><option name="AUTO_RESTART" value="false"/><method v="2"/></configuration>"#
            ),
        )?;
    }
    let kind = if jvm { "jdwp" } else { "inspector" };
    std::fs::write(
        root.join(".aio/dev/debugger.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"kind":kind,"host":"127.0.0.1","port":port}),
        )?,
    )?;
    Ok(())
}

fn write(root: &Path, kind: &str, content: &str) -> Result<()> {
    let directory = root.join(".run");
    std::fs::create_dir_all(&directory)?;
    let ownership = root.join(".aio/dev/ide-files.json");
    let mut files: BTreeMap<String, String> = std::fs::read(&ownership)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let id = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));
    let name = format!("AIO_{kind}_{}.run.xml", &id[..8]);
    let path = directory.join(&name);
    if let Ok(current) = std::fs::read(&path) {
        let digest = format!("{:x}", Sha256::digest(current));
        if files.get(&name) != Some(&digest) {
            eprintln!("保留手工修改的调试配置：{}", path.display());
            return Ok(());
        }
    }
    ensure!(content.contains("<configuration"), "调试配置无效");
    let document =
        format!("<component name=\"ProjectRunConfigurationManager\">\n{content}\n</component>\n");
    std::fs::write(&path, &document)
        .with_context(|| format!("保存调试配置失败: {}", path.display()))?;
    files.insert(name, format!("{:x}", Sha256::digest(document.as_bytes())));
    std::fs::write(ownership, serde_json::to_vec(&files)?)?;
    Ok(())
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn shell(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
