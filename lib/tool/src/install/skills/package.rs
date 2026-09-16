use super::super::execution;
use super::files;
use crate::{CommandSpec, InstallationPlan, publication::Publication};
use anyhow::{Context as _, Result, ensure};
use std::{collections::BTreeMap, fs, path::Path};

// 只识别 AIO 发布协议的结构化 npm 安装步骤，不解析任意 shell 文本。
pub(super) fn packages(plan: &InstallationPlan) -> Result<Vec<Publication>> {
    let mut packages = Vec::new();
    for command in &plan.install {
        if command.program != "npm" {
            continue;
        }
        let args = command.args.iter().map(String::as_str).collect::<Vec<_>>();
        let ["install", "--global", spec] = args.as_slice() else {
            continue;
        };
        let (package, version) = spec.rsplit_once('@').context("npm 安装缺少精确版本")?;
        let publication = Publication {
            package: package.into(),
            version: version.into(),
        };
        publication.validate()?;
        packages.push(publication);
    }
    Ok(packages)
}

pub(super) fn bundled(package: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let root = package.join("skills");
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "包内 skills 必须是普通目录"
        ),
    }
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        ensure!(!entry.file_type()?.is_symlink(), "技能入口不能是符号链接");
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("技能名称不是 UTF-8"))?;
        ensure!(
            crate::validation::identifier(&name) && name.len() <= 64,
            "技能目录名称无效"
        );
        files::collect(&root, &entry.path(), &mut files)?;
        let key = Path::new(&name).join("SKILL.md");
        let bytes = files
            .get(key.to_str().context("技能路径无效")?)
            .context("技能缺少 SKILL.md")?;
        let text = std::str::from_utf8(bytes)?.replace("\r\n", "\n");
        let header = text
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---"))
            .context("SKILL.md 缺少 YAML frontmatter")?
            .0;
        let metadata: serde_yaml::Value = serde_yaml::from_str(header)?;
        ensure!(
            metadata["name"].as_str() == Some(&name)
                && metadata["description"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()),
            "技能名称与目录必须一致，并提供 description"
        );
    }
    Ok(files)
}

pub(super) fn sources(plan: &InstallationPlan) -> Result<BTreeMap<String, Vec<u8>>> {
    let packages = packages(plan)?;
    if packages.is_empty() {
        return Ok(BTreeMap::new());
    }
    let output = execution::output(&CommandSpec {
        program: "npm".into(),
        args: vec!["root".into(), "--global".into()],
    })?;
    let root = Path::new(output.trim());
    ensure!(
        root.is_absolute() && root.is_dir(),
        "npm 没有返回有效的全局安装目录"
    );
    let mut files = BTreeMap::new();
    for publication in packages {
        let package = root.join(&publication.package);
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(package.join("package.json"))?)?;
        ensure!(
            metadata["name"] == publication.package && metadata["version"] == publication.version,
            "已安装 npm 包与指定版本不一致"
        );
        for (relative, bytes) in bundled(&package)? {
            ensure!(
                files.insert(relative, bytes).is_none(),
                "多个 CLI 包提供同名技能"
            );
        }
    }
    Ok(files)
}
