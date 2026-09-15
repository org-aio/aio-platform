use super::*;

#[test]
fn command_and_optional_repository_are_enough_to_register() {
    let mut request = Registration {
        command: "npx -y @scope/my-cli@1.2.3 setup".into(),
        platforms: vec!["macos".into(), "windows".into()],
        ..Default::default()
    };
    let manifest = request.manifest("example".into()).unwrap();
    assert_eq!(manifest.title, "@scope/my-cli");
    assert_eq!(manifest.homepage, "");
    assert!(manifest.platforms["macos"].detect.is_none());
    assert!(manifest.platforms["macos"].uninstall.is_empty());
    assert_eq!(
        manifest.platforms["macos"].install[0].args.last().unwrap(),
        &request.command
    );
    assert_eq!(
        manifest.platforms["windows"].install[0].program,
        "powershell.exe"
    );
    request.metadata.git = "https://github.com/example/tool.git".into();
    assert_eq!(request.manifest("example".into()).unwrap().title, "tool");
    request.metadata.title = "自定义标题".into();
    assert_eq!(
        request.manifest("example".into()).unwrap().title,
        "自定义标题"
    );
    request.metadata.git = "https://user:secret@example.com/repo".into();
    assert!(request.manifest("example".into()).is_err());
}

#[cfg(feature = "native")]
#[test]
fn successful_command_without_detection_is_not_claimed_verified_or_uninstalled()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let request = Registration {
        command: "node --version".into(),
        platforms: vec![std::env::consts::OS.into()],
        ..Default::default()
    };
    let manifest = request.manifest("example".into())?;
    let store = crate::install::Store::new(root.path().into())?;
    store.install(manifest)?;
    assert_eq!(store.read("example")?.unwrap().state, "executed");
    assert!(store.uninstall("example").is_err());
    assert!(store.read("example")?.is_some());
    Ok(())
}

#[cfg(all(feature = "native", unix))]
#[test]
fn failed_download_in_pipeline_is_not_reported_as_success() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let request = Registration {
        command: "false | cat".into(),
        platforms: vec![std::env::consts::OS.into()],
        ..Default::default()
    };
    let store = crate::install::Store::new(root.path().into())?;
    assert!(store.install(request.manifest("example".into())?).is_err());
    assert_eq!(store.read("example")?.unwrap().state, "failed");
    Ok(())
}
