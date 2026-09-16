use anyhow::{Result, ensure};
use include_dir::{Dir, include_dir};
use std::{fs, path::Path};
static TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/templates/plugin/cli");

pub(super) fn materialize(root: &Path, name: &str, title: &str, adopt: bool) -> Result<()> {
    ensure!(name.len() <= 64, "CLI 名称与随包技能名称不能超过 64 字符");
    if !adopt {
        return super::fullstack::materialize_directory(root, &TEMPLATE, name, title);
    }
    let mut package: serde_json::Value =
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
    let skill = root.join("skills").join(name).join("SKILL.md");
    if !skill.exists() {
        let content = TEMPLATE
            .get_file("skills/__NAME__/SKILL.md")
            .and_then(|file| file.contents_utf8())
            .ok_or_else(|| anyhow::anyhow!("CLI 技能模板缺失"))?
            .replace("__NAME__", name);
        fs::create_dir_all(
            skill
                .parent()
                .ok_or_else(|| anyhow::anyhow!("技能路径无效"))?,
        )?;
        super::write(&skill, &content)?;
    }
    if let Some(files) = package["files"].as_array_mut()
        && !files.iter().any(|file| file == "skills")
    {
        files.push("skills".into());
        super::write(
            &root.join("package.json"),
            &(serde_json::to_string_pretty(&package)? + "\n"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adoption_adds_missing_skill_without_replacing_existing_cli() -> Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("package.json"),
            r#"{"name":"existing","bin":{"existing":"bin.mjs"},"files":["bin.mjs"]}"#,
        )?;
        fs::write(root.path().join("bin.mjs"), "console.log('existing')")?;
        fs::write(root.path().join("README.md"), "existing docs")?;
        materialize(root.path(), "existing", "Existing", true)?;
        let package: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.path().join("package.json"))?)?;
        assert_eq!(package["files"], serde_json::json!(["bin.mjs", "skills"]));
        assert!(root.path().join("skills/existing/SKILL.md").is_file());
        assert_eq!(
            fs::read_to_string(root.path().join("bin.mjs"))?,
            "console.log('existing')"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("README.md"))?,
            "existing docs"
        );
        Ok(())
    }

    #[test]
    fn template_is_headless_and_adoption_preserves_existing_source() -> Result<()> {
        let root = tempfile::tempdir()?;
        materialize(root.path(), "my-cli", "工具 \"A\"", false)?;
        let package: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.path().join("package.json"))?)?;
        assert_eq!(package["name"], "my-cli");
        let skill = fs::read_to_string(root.path().join("skills/my-cli/SKILL.md"))?;
        let header = skill
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---"))
            .unwrap()
            .0;
        let metadata: serde_yaml::Value = serde_yaml::from_str(header)?;
        assert_eq!(metadata["name"], "my-cli");
        assert!(
            package["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item == "skills")
        );
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
