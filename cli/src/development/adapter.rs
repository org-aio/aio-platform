use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command};

pub fn build(args: &[String]) -> Result<()> {
    if args == ["rust-frontend"] {
        return super::rust::frontend(&std::env::current_dir()?);
    }
    ensure!(args == ["kotlin-frontend"], "未知开发工具链适配器");
    let root = std::env::current_dir()?;
    let snapshot = root.join(".aio/dev/compiler");
    let output = root.join("build/development");
    // 当前固定的 Kotlin CLI 尚不支持 debug 条件配置；在独立模型中开启 Web 调试，正式构建配置保持原样。
    copy_sources(&root, &snapshot)?;
    if root.join(".aio/toolchain").exists() {
        copy_sources(
            &root.join(".aio/toolchain"),
            &snapshot.join(".aio/toolchain"),
        )?;
    }
    let path = snapshot.join("frontend/module.yaml");
    std::fs::write(
        &path,
        debug_configuration(&std::fs::read_to_string(&path)?)?,
    )?;
    let result = Command::new(snapshot.join("kotlin"))
        .args([
            "build",
            "-m",
            "frontend",
            "-p",
            "wasmJs",
            "-v",
            "debug",
            "--build-dir",
        ])
        .arg(&output)
        .current_dir(&snapshot)
        .status()?;
    ensure!(result.success(), "Kotlin/Wasm 调试构建失败");
    embed_sources(
        &root,
        &output.join("tasks/_frontend_buildWasmJsAppWasmJsDebug/frontend.wasm.map"),
    )?;
    Ok(())
}

fn embed_sources(root: &Path, path: &Path) -> Result<()> {
    let mut map: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sources = map["sources"]
        .as_array()
        .context("Kotlin 源码映射缺少 sources")?
        .clone();
    for (index, source) in sources.iter().enumerate() {
        let Some(relative) = source.as_str().and_then(|source| {
            source
                .split_once(".aio/dev/compiler/")
                .map(|(_, path)| path)
        }) else {
            continue;
        };
        az_plugin_development::validate_relative(relative)?;
        let original = root.join(relative).canonicalize()?;
        ensure!(
            original.starts_with(root),
            "源码映射不能读取工作区以外的文件"
        );
        map["sources"][index] = reqwest::Url::from_file_path(&original)
            .map_err(|_| anyhow::anyhow!("Kotlin 源码路径无效"))?
            .to_string()
            .into();
        map["sourcesContent"][index] = std::fs::read_to_string(original)?.into();
    }
    std::fs::write(path, serde_json::to_vec(&map)?)?;
    Ok(())
}

pub(super) fn copy_sources(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(destination)? {
        let entry = entry?;
        if !source.join(entry.file_name()).exists() {
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|s| {
            matches!(
                s,
                ".aio"
                    | ".git"
                    | ".idea"
                    | "build"
                    | "dist"
                    | "target"
                    | "node_modules"
                    | "aio-dev.lock"
            )
        }) {
            continue;
        }
        let target = destination.join(&name);
        ensure!(
            !entry.file_type()?.is_symlink(),
            "开发快照不能跟随符号链接: {}",
            entry.path().display()
        );
        if entry.file_type()?.is_dir() {
            copy_sources(&entry.path(), &target)?;
        } else {
            if std::fs::read(&target).ok().as_deref()
                != Some(std::fs::read(entry.path())?.as_slice())
            {
                std::fs::copy(entry.path(), &target)
                    .with_context(|| format!("复制开发输入失败: {}", entry.path().display()))?;
            }
        }
    }
    Ok(())
}

fn debug_configuration(text: &str) -> Result<String> {
    use serde_yaml::Value;
    let mut configuration: Value = serde_yaml::from_str(text)?;
    let settings = configuration
        .as_mapping_mut()
        .context("Kotlin 模块必须是映射")?
        .entry(Value::from("settings"))
        .or_insert(Value::Mapping(Default::default()));
    let kotlin = settings
        .as_mapping_mut()
        .context("settings 必须是映射")?
        .entry(Value::from("kotlin"))
        .or_insert(Value::Mapping(Default::default()));
    let args = kotlin
        .as_mapping_mut()
        .context("settings.kotlin 必须是映射")?
        .entry(Value::from("freeCompilerArgs"))
        .or_insert(Value::Sequence(vec![]))
        .as_sequence_mut()
        .context("freeCompilerArgs 必须是列表")?;
    args.retain(|argument| {
        !argument.as_str().is_some_and(|argument| {
            argument.starts_with("-source-map-embed-sources=")
                && argument != "-source-map-embed-sources=always"
        })
    });
    for argument in [
        "-source-map",
        "-source-map-embed-sources=always",
        "-Xwasm-debug-info",
        "-Xwasm-debug-friendly",
        "-Xwasm-debugger-custom-formatters",
    ] {
        if !args.contains(&Value::from(argument)) {
            args.push(Value::from(argument));
        }
    }
    Ok(serde_yaml::to_string(&configuration)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn development_flags_preserve_custom_compiler_arguments() -> Result<()> {
        let original = "product: lib\nsettings:\n  kotlin:\n    freeCompilerArgs: ['-Xexpect-actual-classes', '-source-map-embed-sources=never']\n";
        let changed = debug_configuration(original)?;
        assert!(changed.contains("-Xexpect-actual-classes"));
        assert!(changed.contains("-source-map-embed-sources=always"));
        assert!(!changed.contains("=never"));
        assert_eq!(debug_configuration(&changed)?, changed);
        Ok(())
    }
}
