use super::*;

fn project(root: &Path, name: &str, dependencies: &[(&str, &str)]) -> Result<PathBuf> {
    let path = root.join(name);
    std::fs::create_dir_all(&path)?;
    let mut manifest = String::from("[plugin]\n");
    for (dependency, version) in dependencies {
        manifest.push_str(&format!("[[plugin.dependencies]]\ngit='https://github.com/example/{dependency}.git'\nversion='{version}'\n"));
    }
    std::fs::write(path.join("aio-plugin.toml"), manifest)?;
    std::fs::write(
        path.join("Cargo.toml"),
        "[workspace.package]\nversion='1.2.0'\n",
    )?;
    std::fs::write(
        path.join("aio-dev.toml"),
        "version=1\n[frontend]\ninputs=['frontend']\ncommand=['true']\noutput='dist/web'\n[backend]\ninputs=['backend']\ncommand=['true']\noutput='dist/plugin.wasm'\n",
    )?;
    ensure!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&path)
            .status()?
            .success(),
        "测试仓库初始化失败"
    );
    ensure!(
        Command::new("git")
            .arg("-C")
            .arg(&path)
            .args([
                "remote",
                "add",
                "origin",
                &format!("https://github.com/example/{name}.git")
            ])
            .status()?
            .success(),
        "测试来源初始化失败"
    );
    path.canonicalize().map_err(Into::into)
}

#[test]
fn loads_only_transitive_dependencies_in_dependency_order() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = project(temp.path(), "root", &[("middle", "^1")])?;
    let middle = project(temp.path(), "middle", &[("leaf", "~1.2")])?;
    let leaf = project(temp.path(), "leaf", &[])?;
    project(temp.path(), "unrelated", &[])?;
    let (workspaces, lock) = resolve(&root, &[middle, leaf], true)?;
    assert_eq!(
        workspaces
            .iter()
            .map(|w| w.root.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        ["leaf", "middle", "root"]
    );
    assert_eq!(lock.plugins.len(), 3);
    assert!(lock.plugins.iter().all(|p| p.source_sha.is_none()
        && p.package_digest.is_none()
        && p.version.as_deref() == Some("1.2.0")));
    Ok(())
}

#[test]
fn rejects_missing_conflicting_cyclic_and_undeclared_dependencies() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let leaf = project(temp.path(), "leaf", &[])?;
    let missing = project(temp.path(), "missing", &[("leaf", "^1")])?;
    assert!(
        resolve(&missing, &[], true)
            .unwrap_err()
            .to_string()
            .contains("--with")
    );
    let conflict = project(temp.path(), "conflict", &[("leaf", "^2")])?;
    assert!(
        resolve(&conflict, std::slice::from_ref(&leaf), true)
            .unwrap_err()
            .to_string()
            .contains("版本冲突")
    );
    let first = project(temp.path(), "first", &[("second", "*")])?;
    let second = project(temp.path(), "second", &[("first", "*")])?;
    assert!(
        resolve(&first, &[second], true)
            .unwrap_err()
            .to_string()
            .contains("循环")
    );
    assert!(
        resolve(&leaf, &[missing], true)
            .unwrap_err()
            .to_string()
            .contains("未声明")
    );
    Ok(())
}

#[test]
fn uncommitted_projects_do_not_inherit_the_parent_repository() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let parent = project(temp.path(), "parent", &[])?;
    let root = project(&parent, "root", &[("leaf", "^1")])?;
    let leaf = project(&parent, "leaf", &[])?;
    std::fs::remove_dir_all(root.join(".git"))?;
    std::fs::remove_dir_all(leaf.join(".git"))?;
    let (workspaces, lock) = resolve(&root, &[leaf], true)?;
    assert_eq!(workspaces.len(), 2);
    assert_eq!(
        lock.plugins[0].source,
        "https://github.com/example/leaf.git"
    );
    assert!(lock.plugins[1].source.starts_with("file://"));
    assert!(
        lock.plugins
            .iter()
            .all(|plugin| plugin.source_sha.is_none())
    );
    Ok(())
}
