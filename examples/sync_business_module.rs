use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use az_studio::{
    ApplicationCompiler, ApplicationWorkspace, BusinessModuleManager, CapabilityCatalog,
    ImageTarget, ProgramCompiler, ProgramDefinition,
};

fn main() -> Result<()> {
    let definition_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .context("用法: sync_business_module <program-definition.json>")?;
    let definition = serde_json::from_slice::<ProgramDefinition>(
        &fs::read(&definition_path)
            .with_context(|| format!("读取程序定义失败: {}", definition_path.display()))?,
    )
    .context("解析程序定义失败")?;
    let result = BusinessModuleManager::repository().reconcile(&definition)?;
    let image = ProgramCompiler::new("workspace-sync", &CapabilityCatalog::default())
        .compile(&definition, "workspace-sync", ImageTarget::Universal)
        .map_err(anyhow::Error::from)?;
    let application = ApplicationCompiler.compile(&definition, &image)?;
    let application = ApplicationWorkspace::repository().write(&application)?;
    println!(
        "已同步 lib/biz/{}：{} 个生成文件，{} 个新 Service 实现槽",
        result.business_module,
        result.generated_files.len(),
        result.created_service_implementations.len()
    );
    println!(
        "已同步 generated/apps/{}：{} 个文件",
        application.application_id,
        application.files.len()
    );
    Ok(())
}
