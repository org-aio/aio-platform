use anyhow::{Result, ensure};

use crate::DeliveryManifest;

pub fn parse(source: &str) -> Result<DeliveryManifest> {
    ensure!(source.len() <= 16384, "交付清单过大");
    let manifest: DeliveryManifest = toml::from_str(source)?;
    ensure!(manifest.version == 1, "不支持的交付协议版本");
    validate_recipe(&manifest.build)?;
    Ok(manifest)
}

/// 发现插件提交构建配方时复用与文件清单相同的参数限制。
pub fn validate_recipe(recipe: &crate::BuildRecipe) -> Result<()> {
    ensure!(!recipe.command.is_empty(), "缺少构建命令");
    ensure!(recipe.command.len() <= 32, "构建参数过多");
    ensure!(
        recipe
            .command
            .iter()
            .all(|arg| !arg.contains('\0') && arg.len() <= 4096),
        "构建参数无效"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_protocol_and_empty_commands() {
        for source in [
            "version=2\n[build]\nenvironment='rust'\ncommand=['sh','build.sh']",
            "version=1\n[build]\nenvironment='kotlin'\ncommand=[]",
            "version=1\n[build]\nenvironment='typescript'\ncommand=['x']\nprivileged=true",
        ] {
            assert!(parse(source).is_err());
        }
    }
}
