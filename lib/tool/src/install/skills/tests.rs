use super::*;
use super::{
    deployment::apply,
    package::{bundled, packages},
};
use crate::CommandSpec;
use std::{collections::BTreeMap, fs, path::Path};

fn record() -> Result<Installation> {
    let manifest = serde_json::from_str(include_str!(
        "../../../../../tools/registry/codex-model-sync-0.4.1.json"
    ))?;
    Ok(Installation {
        manifest,
        state: "installing".into(),
        completed_steps: 0,
        skills: BTreeMap::new(),
    })
}

fn package(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("skills/example/references"))?;
    fs::write(
        root.join("skills/example/SKILL.md"),
        "---\nname: example\ndescription: CLI fixture usage.\n---\n# example\n",
    )?;
    fs::write(root.join("skills/example/references/usage.md"), "usage")?;
    Ok(())
}

#[test]
fn install_tracks_files_and_uninstall_preserves_user_edits() -> Result<()> {
    let root = tempfile::tempdir()?;
    package(root.path())?;
    let store = Store::new(root.path().join("state"))?;
    let mut record = record()?;
    apply(&store, bundled(root.path())?, &mut record)?;
    assert_eq!(record.skills.len(), 2);
    assert!(store.read(&record.manifest.id)?.unwrap().skills.len() == 2);
    apply(&store, bundled(root.path())?, &mut record)?;
    let recorded = record.skills.clone();
    let mut changed = bundled(root.path())?;
    changed.insert(
        "example/references/usage.md".into(),
        b"changed release".to_vec(),
    );
    assert!(apply(&store, changed, &mut record).is_err());
    assert_eq!(record.skills, recorded);
    assert_eq!(store.read(&record.manifest.id)?.unwrap().skills, recorded);
    let target = store.skills_root.join("example/SKILL.md");
    fs::write(&target, "user edit")?;
    assert!(apply(&store, bundled(root.path())?, &mut record).is_err());
    uninstall(&store, &record)?;
    assert_eq!(fs::read_to_string(target)?, "user edit");
    assert!(!store.skills_root.join("example/references").exists());
    uninstall(&store, &record)?;
    Ok(())
}

#[test]
fn unrelated_skills_and_invalid_metadata_are_not_overwritten() -> Result<()> {
    let root = tempfile::tempdir()?;
    package(root.path())?;
    let store = Store::new(root.path().join("state"))?;
    fs::create_dir_all(store.skills_root.join("example"))?;
    fs::write(store.skills_root.join("example/SKILL.md"), "existing")?;
    let mut record = record()?;
    assert!(apply(&store, bundled(root.path())?, &mut record).is_err());
    assert!(record.skills.is_empty());
    assert_eq!(
        fs::read_to_string(store.skills_root.join("example/SKILL.md"))?,
        "existing"
    );
    fs::write(
        root.path().join("skills/example/SKILL.md"),
        "---\nname: other\ndescription: mismatch\n---\n",
    )?;
    assert!(bundled(root.path()).is_err());
    assert!(files::destination(&store.skills_root, "../outside").is_err());
    Ok(())
}

#[test]
fn only_structured_exact_npm_installs_provide_package_sources() -> Result<()> {
    let record = record()?;
    let mut plan = record.manifest.platforms["macos"].clone();
    let actual = packages(&plan)?;
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0].package, "codex-model-sync");
    plan.install = vec![CommandSpec {
        program: "bash".into(),
        args: vec!["-c".into(), "npm install --global arbitrary".into()],
    }];
    assert!(packages(&plan)?.is_empty());
    plan.install = vec![CommandSpec {
        program: "npm".into(),
        args: vec![
            "install".into(),
            "--global".into(),
            "@scope/tool@1.2.3".into(),
        ],
    }];
    assert_eq!(packages(&plan)?[0].package, "@scope/tool");
    plan.install[0].args[2] = "../outside@1.2.3".into();
    assert!(packages(&plan).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_read_or_write_outside_the_skill_package() -> Result<()> {
    let root = tempfile::tempdir()?;
    package(root.path())?;
    let secret = root.path().join("private");
    fs::write(&secret, "private")?;
    std::os::unix::fs::symlink(&secret, root.path().join("skills/example/leak"))?;
    assert!(bundled(root.path()).is_err());
    let target = root.path().join("target");
    fs::create_dir_all(&target)?;
    std::os::unix::fs::symlink(root.path(), target.join("example"))?;
    assert!(files::destination(&target, "example/private").is_err());
    Ok(())
}
