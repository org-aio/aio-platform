use anyhow::{Context, Result, ensure};
use az_plugin_enhancement::Package;

/// 本地验证 AI 生成的候选包，不连接宿主或修改活动版本。
fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .context("用法: verify <增强包.json>")?;
    ensure!(
        std::fs::metadata(&path)?.len() <= az_plugin_enhancement::MAX_PACKAGE_BYTES as u64,
        "增强包超过配额"
    );
    let package: Package = serde_json::from_slice(&std::fs::read(path)?)?;
    let prepared = az_plugin_enhancement::default_runtime()?.prepare(package)?;
    println!(
        "{}",
        serde_json::json!({"revision":prepared.revision(),"paths":prepared.package().enhancements.iter().map(|item| &item.path).collect::<Vec<_>>()})
    );
    Ok(())
}
