use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command};

pub(super) fn frontend(root: &Path) -> Result<()> {
    let manifest: toml::Value =
        toml::from_str(&std::fs::read_to_string(root.join("frontend/Cargo.toml"))?)?;
    let package = manifest["package"]["name"]
        .as_str()
        .context("Rust 前端缺少 package.name")?;
    let public = root.join(format!("target/dx/{package}/debug/web/public"));
    let cache = root.join(".aio/dev");
    std::fs::create_dir_all(&cache)?;
    let style = cache.join("shared.css");
    let marker = cache.join("rust-style-digest");
    let inputs = [
        "Cargo.toml".into(),
        "Cargo.lock".into(),
        "frontend/Cargo.toml".into(),
    ];
    let digest = az_plugin_development::fingerprint(root, &inputs)?;
    if !style.is_file() || std::fs::read_to_string(&marker).ok().as_deref() != Some(&digest) {
        build(root, package, true)?;
        let mut css = Vec::new();
        stylesheets(&public.join("assets"), &mut css)?;
        std::fs::write(&style, css)?;
        std::fs::write(&marker, az_plugin_development::fingerprint(root, &inputs)?)?;
    }
    build(root, package, false)?;
    let destination = root.join("dist/web");
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    super::adapter::copy_sources(&public.join("wasm"), &destination.join("assets"))?;
    let javascript = destination.join(format!("assets/{package}.js"));
    let text = std::fs::read_to_string(&javascript)?;
    let startup = format!("module_or_path: \"/./wasm/{package}_bg.wasm\"");
    ensure!(
        text.contains(&startup),
        "Dioxus 调试入口格式已变化，请更新固定工具链适配器"
    );
    // 把编译器生成的应用根地址绑定到当前模块，保留 iframe 资源隔离与调试符号。
    std::fs::write(
        javascript,
        text.replace(
            &startup,
            &format!("module_or_path: new URL(\"./{package}_bg.wasm\", import.meta.url)"),
        ),
    )?;
    std::fs::copy(
        root.join("frontend/index.html"),
        destination.join("index.html"),
    )?;
    std::fs::copy(style, destination.join("assets/shared.css"))?;
    std::fs::write(
        destination.join("bootstrap.js"),
        format!("import \"./assets/{package}.js\";\n"),
    )?;
    Ok(())
}

fn build(root: &Path, package: &str, styles: bool) -> Result<()> {
    let mut command = Command::new("dx");
    command.current_dir(root).args([
        "build",
        "--package",
        package,
        "--web",
        "--inject-loading-scripts",
        "false",
    ]);
    if styles {
        command.args(["--features", "bundle-assets"]);
    }
    ensure!(command.status()?.success(), "Rust 前端 debug 构建失败");
    Ok(())
}

fn stylesheets(root: &Path, css: &mut Vec<u8>) -> Result<()> {
    let mut entries = std::fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            stylesheets(&path, css)?;
        } else if path.extension().is_some_and(|extension| extension == "css") {
            css.extend(std::fs::read(path)?);
            css.push(b'\n');
        }
    }
    Ok(())
}
