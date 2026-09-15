use super::*;

fn sample() -> ToolManifest {
    serde_json::from_str(include_str!(
        "../../../tools/registry/codex-model-sync-0.1.4.json"
    ))
    .unwrap()
}

#[test]
fn links_cannot_change_commands_origins_or_confirmation() {
    let input = "aio://install/codex-model-sync?version=0.1.4";
    assert_eq!(InstallLink::parse(input).unwrap().to_string(), input);
    let build = InstallLink {
        id: "tool".into(),
        version: "1.0.0+build.1".into(),
    };
    assert_eq!(InstallLink::parse(&build.to_string()).unwrap(), build);
    for input in [
        "aio://install/codex-model-sync?version=0.1.4&yes=true",
        "aio://install/codex-model-sync?version=0.1.4&version=0.2.0",
        "aio://install/codex-model-sync?version=latest",
        "aio://install/codex-model-sync?version=0.1.4#x",
        "aio://evil/codex-model-sync?version=0.1.4",
        "aio://install/a/../codex-model-sync?version=0.1.4",
        "aio://install/%63odex-model-sync?version=0.1.4",
        "aio://install/codex-model-sync?version=0.1.4&origin=https://evil.test",
    ] {
        assert!(InstallLink::parse(input).is_err(), "{input}");
    }
}

#[test]
fn cli_accepts_optional_uninstall_but_rejects_invalid_identifiers() {
    let mut value = sample();
    value.validate().unwrap();
    value.platforms.get_mut("macos").unwrap().uninstall.clear();
    value.validate().unwrap();
    let mut value = sample();
    value.id = "../escape".into();
    assert!(value.validate().is_err());
}

#[cfg(feature = "native")]
#[test]
fn failed_install_keeps_original_uninstall_plan_and_stops_following_steps() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let marker = directory.path().join("quote ' and space.txt");
    let write = CommandSpec {
        program: "node".into(),
        args: vec![
            "-e".into(),
            "require('fs').writeFileSync(process.argv[1], 'installed')".into(),
            marker.to_string_lossy().into(),
        ],
    };
    let fail = CommandSpec {
        program: "node".into(),
        args: vec!["-e".into(), "process.exit(7)".into()],
    };
    let remove = CommandSpec {
        program: "node".into(),
        args: vec![
            "-e".into(),
            "require('fs').rmSync(process.argv[1], {force:true})".into(),
            marker.to_string_lossy().into(),
        ],
    };
    let mut manifest = sample();
    manifest.id = "isolated-test".into();
    let plan = manifest.platforms.get_mut(std::env::consts::OS).unwrap();
    plan.requirements.clear();
    plan.install = vec![write, fail.clone()];
    plan.uninstall = vec![remove];
    plan.detect = Some(fail);
    let store = install::Store::new(directory.path().join("state"))?;
    assert!(store.install(manifest.clone()).is_err());
    assert!(marker.exists());
    let record = store.read(&manifest.id)?.unwrap();
    assert_eq!(record.state, "failed");
    assert_eq!(record.completed_steps, 1);
    let mut changed = manifest.clone();
    changed.version = "0.1.5".into();
    assert!(store.install(changed).is_err());
    store.uninstall(&manifest.id)?;
    assert!(!marker.exists());
    assert!(store.read(&manifest.id)?.is_none());
    Ok(())
}

#[cfg(feature = "native")]
#[test]
fn successful_commands_require_detection_before_install_is_marked_complete() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let mut manifest = sample();
    let plan = manifest.platforms.get_mut(std::env::consts::OS).unwrap();
    plan.requirements.clear();
    let succeed = CommandSpec {
        program: "node".into(),
        args: vec!["-e".into(), "process.exit(0)".into()],
    };
    plan.install = vec![succeed.clone()];
    plan.uninstall = vec![succeed.clone()];
    plan.detect = Some(succeed);
    let store = install::Store::new(directory.path().join("state"))?;
    store.install(manifest.clone())?;
    assert_eq!(store.read(&manifest.id)?.unwrap().state, "installed");
    store.uninstall(&manifest.id)?;
    assert!(store.list()?.is_empty());
    Ok(())
}

#[cfg(feature = "native")]
#[test]
fn uninstall_retry_does_not_repeat_completed_restore_steps() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let restored = directory.path().join("restored");
    let ready = directory.path().join("ready");
    let node = |script: &str, path: &std::path::Path| CommandSpec {
        program: "node".into(),
        args: vec!["-e".into(), script.into(), path.to_string_lossy().into()],
    };
    let mut manifest = sample();
    let plan = manifest.platforms.get_mut(std::env::consts::OS).unwrap();
    plan.requirements.clear();
    plan.install = vec![CommandSpec {
        program: "node".into(),
        args: vec!["--version".into()],
    }];
    plan.detect = Some(plan.install[0].clone());
    plan.uninstall = vec![
        node(
            "require('fs').writeFileSync(process.argv[1], 'restored', {flag:'wx'})",
            &restored,
        ),
        node(
            "process.exit(require('fs').existsSync(process.argv[1]) ? 0 : 7)",
            &ready,
        ),
    ];
    let store = install::Store::new(directory.path().join("state"))?;
    store.install(manifest.clone())?;
    assert!(store.uninstall(&manifest.id).is_err());
    let record = store.read(&manifest.id)?.unwrap();
    assert_eq!(record.state, "uninstalling");
    assert_eq!(record.completed_steps, 1);
    std::fs::write(&ready, "ready")?;
    store.uninstall(&manifest.id)?;
    assert!(store.read(&manifest.id)?.is_none());
    Ok(())
}

#[cfg(feature = "native")]
#[test]
fn fetch_rejects_redirects_wrong_versions_and_oversized_descriptions() -> anyhow::Result<()> {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let manifest = sample();
    let link = InstallLink {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
    };
    let good = serde_json::to_vec(&manifest)?;
    let mut wrong = manifest;
    wrong.version = "9.9.9".into();
    for (status, body, success) in [
        (200, good.clone(), true),
        (302, good, false),
        (200, serde_json::to_vec(&wrong)?, false),
        (200, vec![b' '; MAX_MANIFEST_BYTES as usize + 1], false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/manifest", listener.local_addr()?);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).unwrap();
            assert!(
                !String::from_utf8_lossy(&request[..count])
                    .to_lowercase()
                    .contains("authorization:")
            );
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            let _ = stream.write_all(&body);
        });
        assert_eq!(install::fetch_from(&url, &link).is_ok(), success);
        server.join().unwrap();
    }
    Ok(())
}
