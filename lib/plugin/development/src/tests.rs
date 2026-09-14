use super::*;

#[test]
fn artifact_identity_is_content_based_and_independent_of_output_layout() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let make = |name: &str| -> anyhow::Result<_> {
        let root = temp.path().join(name);
        std::fs::create_dir_all(root.join("frontend"))?;
        std::fs::write(root.join("manifest"), "[plugin]")?;
        std::fs::write(root.join("frontend/index.html"), "old")?;
        std::fs::write(root.join("backend"), "component")?;
        Ok(root)
    };
    let first = make("build")?;
    let second = make("immutable")?;
    let digest = |root: &std::path::Path| {
        artifact_digest(
            &root.join("manifest"),
            &root.join("frontend"),
            &root.join("backend"),
        )
    };
    assert_eq!(digest(&first)?, digest(&second)?);
    let backend = backend_digest(&first.join("backend"))?;
    std::fs::write(first.join("frontend/index.html"), "new")?;
    assert_ne!(digest(&first)?, digest(&second)?);
    assert_eq!(backend_digest(&first.join("backend"))?, backend);
    std::fs::write(first.join("frontend/new.map"), "map")?;
    let added = digest(&first)?;
    std::fs::remove_file(first.join("frontend/new.map"))?;
    assert_ne!(added, digest(&first)?);
    Ok(())
}

#[test]
fn source_changes_and_deletions_invalidate_inputs_but_generated_state_does_not()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    std::fs::write(temp.path().join("source.kt"), "one")?;
    let inputs = [".".into()];
    let original = fingerprint(temp.path(), &inputs)?;
    std::fs::create_dir(temp.path().join(".aio"))?;
    std::fs::write(temp.path().join(".aio/state"), "generated")?;
    std::fs::write(temp.path().join("aio-dev.lock"), "lock")?;
    assert_eq!(fingerprint(temp.path(), &inputs)?, original);
    std::fs::write(temp.path().join("source.kt"), "two")?;
    assert_ne!(fingerprint(temp.path(), &inputs)?, original);
    std::fs::remove_file(temp.path().join("source.kt"))?;
    assert_ne!(fingerprint(temp.path(), &inputs)?, original);
    assert!(validate_relative("../outside").is_err());
    Ok(())
}

#[test]
fn successful_automatic_versions_are_selectable_and_pinned() -> anyhow::Result<()> {
    use crate::{DependencyCandidate, resolve_dependencies};
    let result = resolve_dependencies("root", |source| {
        Ok(vec![DependencyCandidate {
            source: source.into(),
            version: "0.0.0-dev.12+0123456789".parse()?,
            dependencies: vec![],
        }])
    })?;
    assert_eq!(result[0].version.to_string(), "0.0.0-dev.12+0123456789");
    assert!(crate::matches_requirement(
        &"=0.0.0-dev.12".parse()?,
        &result[0].version
    ));
    assert!(!crate::matches_requirement(
        &"^1.0".parse()?,
        &result[0].version
    ));
    Ok(())
}
