use anyhow::{Result, ensure};
use include_dir::{Dir, include_dir};
use std::{fs, path::Path};
static TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/cli");

pub(super) fn materialize(root: &Path, name: &str, title: &str, adopt: bool) -> Result<()> {
    if !adopt {
        return super::fullstack::materialize_directory(root, &TEMPLATE, name, title);
    }
    let package: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("package.json"))?)?;
    let bin = package["bin"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("现有 CLI 必须声明 package.json bin 对象"))?;
    ensure!(bin.len() == 1, "存在多个命令，请先明确需要上架的 CLI 入口");
    let command = bin.keys().next().unwrap();
    let config = az_tool::publication::CliConfiguration {
        id: name.into(),
        title: title.into(),
        command: command.into(),
        platforms: vec!["macos".into(), "linux".into(), "windows".into()],
        setup: vec![],
        uninstall: vec![],
    };
    let files = ["aio-cli.json", "AIO.md", ".github/workflows/aio-cli.yml"];
    for file in files {
        ensure!(
            !root.join(file).exists(),
            "接入文件已存在，拒绝覆盖: {file}"
        );
    }
    super::write(
        &root.join("aio-cli.json"),
        &(serde_json::to_string_pretty(&config)? + "\n"),
    )?;
    for file in ["AIO.md", ".github/workflows/aio-cli.yml"] {
        let destination = root.join(file);
        fs::create_dir_all(destination.parent().unwrap())?;
        let content = TEMPLATE
            .get_file(file)
            .unwrap()
            .contents_utf8()
            .unwrap()
            .replace("__AIO_VERSION__", env!("CARGO_PKG_VERSION"));
        super::write(&destination, &content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_is_headless_and_adoption_preserves_existing_source() -> Result<()> {
        let root = tempfile::tempdir()?;
        materialize(root.path(), "my-cli", "工具 \"A\"", false)?;
        let package: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.path().join("package.json"))?)?;
        assert_eq!(package["name"], "my-cli");
        assert!(!root.path().join("aio-plugin.toml").exists());
        assert!(
            fs::read_to_string(root.path().join(".github/workflows/aio-cli.yml"))?
                .contains("tool release sync")
        );
        let original = fs::read(root.path().join("src/cli.ts"))?;
        assert!(materialize(root.path(), "my-cli", "工具", true).is_err());
        assert_eq!(fs::read(root.path().join("src/cli.ts"))?, original);
        for file in ["aio-cli.json", "AIO.md", ".github/workflows/aio-cli.yml"] {
            fs::remove_file(root.path().join(file))?;
        }
        materialize(root.path(), "my-cli", "工具", true)?;
        assert_eq!(fs::read(root.path().join("src/cli.ts"))?, original);
        Ok(())
    }
}
